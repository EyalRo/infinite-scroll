//! BlueZ GATT server: advertising, characteristics, and the glue between
//! the framed byte streams and `Handler`. Nothing here knows what any
//! operation means.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bluer::adv::Advertisement;
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotify, CharacteristicNotifyMethod, CharacteristicNotifier, CharacteristicRead,
    CharacteristicWrite, CharacteristicWriteMethod, Service,
};
use futures::FutureExt;
use serde_json::json;
use tokio::sync::Mutex;

use crate::handler::{Backend, Handler};
use crate::protocol::*;

/// ATT notification payload = MTU - 3 bytes of ATT header, but never more than
/// MAX_ATTR_VALUE: Android drops characteristic values over 512 bytes.
const ATT_OVERHEAD: usize = 3;
const MAX_ATTR_VALUE: usize = 512;
const POLL_INTERVAL: Duration = Duration::from_secs(2);

struct Shared<B: Backend> {
    handler: Arc<Handler<B>>,
    reassembler: StdMutex<Reassembler>,
    /// Last ATT MTU seen on a write; the response chunk size derives from it.
    mtu: AtomicU16,
    response: Mutex<Option<CharacteristicNotifier>>,
    changed: Mutex<Option<CharacteristicNotifier>>,
    revision: AtomicU32,
    local_name: String,
}

impl<B: Backend + 'static> Shared<B> {
    fn chunk_payload(&self) -> usize {
        let value = (self.mtu.load(Ordering::Relaxed) as usize).saturating_sub(ATT_OVERHEAD).min(MAX_ATTR_VALUE);
        value.saturating_sub(FRAME_HEADER_LEN).max(1)
    }

    async fn send_response(&self, msg_id: u16, body: Vec<u8>) {
        let frames = encode_frames(msg_id, &body, self.chunk_payload());
        // Hold the notifier for the whole message so frames of different
        // responses never interleave.
        let mut guard = self.response.lock().await;
        let Some(notifier) = guard.as_mut() else {
            log::warn!("response {msg_id} dropped: client is not subscribed");
            return;
        };
        for frame in frames {
            if let Err(error) = notifier.notify(frame).await {
                log::warn!("response {msg_id} aborted: {error}");
                *guard = None;
                return;
            }
        }
    }

    async fn notify_changed(&self, mask: u16) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let mut payload = mask.to_le_bytes().to_vec();
        payload.extend_from_slice(&revision.to_le_bytes());
        let mut guard = self.changed.lock().await;
        if let Some(notifier) = guard.as_mut() {
            if notifier.notify(payload).await.is_err() {
                *guard = None;
            }
        }
    }

    fn info_json(&self) -> Vec<u8> {
        json!({
            "proto": PROTOCOL_VERSION,
            "name": self.local_name,
            "app": env!("CARGO_PKG_VERSION"),
            "max_upload_bytes": common::MAX_UPLOAD_BYTES,
            "domains": {"status": DOMAIN_STATUS, "library": DOMAIN_LIBRARY, "printer": DOMAIN_PRINTER, "scheduler": DOMAIN_SCHEDULER},
        })
        .to_string()
        .into_bytes()
    }
}

pub async fn run<B: Backend + 'static>(handler: Arc<Handler<B>>, local_name: String) -> bluer::Result<()> {
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    log::info!("using adapter {} ({})", adapter.name(), adapter.address().await?);

    let shared = Arc::new(Shared {
        handler,
        reassembler: StdMutex::new(Reassembler::default()),
        mtu: AtomicU16::new(23),
        response: Mutex::new(None),
        changed: Mutex::new(None),
        revision: AtomicU32::new(0),
        local_name: local_name.clone(),
    });

    let info = {
        let shared = shared.clone();
        Characteristic {
            uuid: INFO_UUID,
            read: Some(CharacteristicRead {
                read: true,
                fun: Box::new(move |_req| {
                    let value = shared.info_json();
                    async move { Ok(value) }.boxed()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    };

    let request = {
        let shared = shared.clone();
        Characteristic {
            uuid: REQUEST_UUID,
            write: Some(CharacteristicWrite {
                write: true,
                write_without_response: true,
                method: CharacteristicWriteMethod::Fun(Box::new(move |frame, req| {
                    let shared = shared.clone();
                    async move {
                        shared.mtu.store(req.mtu, Ordering::Relaxed);
                        let assembled = shared.reassembler.lock().unwrap_or_else(|p| p.into_inner()).push(&frame);
                        match assembled {
                            Ok(Some((msg_id, body))) => {
                                tokio::spawn(async move {
                                    let handler = shared.handler.clone();
                                    let response = tokio::task::spawn_blocking(move || handler.handle_message(&body))
                                        .await
                                        .unwrap_or_else(|_| error_response(msg_id as u64, ErrorCode::Internal, "handler panicked", None));
                                    shared.send_response(msg_id, response.to_string().into_bytes()).await;
                                });
                            }
                            Ok(None) => {}
                            Err(error) => log::warn!("bad request frame: {error:?}"),
                        }
                        Ok(())
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    };

    let response = {
        let shared = shared.clone();
        Characteristic {
            uuid: RESPONSE_UUID,
            notify: Some(CharacteristicNotify {
                notify: true,
                method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
                    let shared = shared.clone();
                    async move {
                        log::info!("client subscribed to responses");
                        *shared.response.lock().await = Some(notifier);
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    };

    let changed = {
        let shared = shared.clone();
        Characteristic {
            uuid: CHANGED_UUID,
            notify: Some(CharacteristicNotify {
                notify: true,
                method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
                    let shared = shared.clone();
                    async move {
                        *shared.changed.lock().await = Some(notifier);
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    };

    let upload = {
        let shared = shared.clone();
        Characteristic {
            uuid: UPLOAD_UUID,
            write: Some(CharacteristicWrite {
                write_without_response: true,
                write: true,
                method: CharacteristicWriteMethod::Fun(Box::new(move |bytes, _req| {
                    let shared = shared.clone();
                    async move {
                        shared.handler.upload_chunk(&bytes);
                        Ok(())
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    };

    let app = Application {
        services: vec![Service {
            uuid: SERVICE_UUID,
            primary: true,
            characteristics: vec![info, request, response, changed, upload],
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app_handle = adapter.serve_gatt_application(app).await?;

    let advertisement = Advertisement {
        service_uuids: BTreeSet::from([SERVICE_UUID]),
        discoverable: Some(true),
        local_name: Some(local_name),
        ..Default::default()
    };
    let _adv_handle = adapter.advertise(advertisement).await?;
    log::info!("GATT service {SERVICE_UUID} advertising");

    // Change detection: poll the services and notify the phone which
    // domains moved, so it can re-request just those.
    let poller = shared.clone();
    let poll_task = tokio::spawn(async move {
        let mut previous: Option<[u64; 4]> = None;
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        loop {
            ticker.tick().await;
            if poller.changed.lock().await.is_none() {
                previous = None; // re-baseline when a client (re)subscribes
                continue;
            }
            let handler = poller.handler.clone();
            let Ok(current) = tokio::task::spawn_blocking(move || handler.domain_digests()).await else { continue };
            if let Some(prev) = previous {
                let bits = [DOMAIN_STATUS, DOMAIN_LIBRARY, DOMAIN_PRINTER, DOMAIN_SCHEDULER];
                let mask = (0..4).filter(|&i| prev[i] != current[i]).fold(0u16, |m, i| m | bits[i]);
                if mask != 0 {
                    poller.notify_changed(mask).await;
                }
            }
            previous = Some(current);
        }
    });

    tokio::signal::ctrl_c().await.ok();
    poll_task.abort();
    log::info!("shutting down");
    Ok(())
}
