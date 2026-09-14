# Raspberry Pi installation bring-up

Validated 2026-09-13 for the *Infinite Scroll* installation.

## Hardware and operating system

- Raspberry Pi 4 Model B Rev 1.2
- 128 GB microSD card
- Raspberry Pi OS Lite 64-bit, Debian 13 (trixie)
- Hostname: `infinite-scroll`
- Administrative user: `stags`
- SSH key authentication only; password login disabled
- Wi-Fi and regulatory region preconfigured during imaging
- Timezone: `America/Los_Angeles`; locale: `en_US.UTF-8`

Do not commit Wi-Fi credentials, private SSH keys, cloud-init seed files, or
host-specific secrets. Use Raspberry Pi Imager customization to inject them.

## Imaging procedure

The card was written with Raspberry Pi Imager 2.0.9 in CLI mode using the
official `raspios_lite_arm64` image. The compressed image was checked against
Raspberry Pi's published SHA-256 before writing. Imager performed its own
full device read-back verification after writing and injected cloud-init
`user-data` and `network-config` files.

Important checksum detail: Imager's `--sha256` option compares against the
decompressed image stream, while the Raspberry Pi download directory
publishes a checksum for the `.img.xz` file. Verify the downloaded `.xz`
separately with `sha256sum -c`; do not pass that compressed-file hash to
Imager as though it were the decompressed-image hash.

Cloud-init created the `stags` account, installed its SSH public key, enabled
passwordless sudo for installation administration, disabled SSH password
authentication, set the hostname/locale/timezone, and enabled `ssh`.

## First-boot verification

The Pi appeared through mDNS as `infinite-scroll.local` and initially leased
`192.168.0.111`. Treat the address as dynamic; prefer the mDNS hostname.

```sh
ssh stags@infinite-scroll.local
hostname
tr -d '\000' </proc/device-tree/model
cloud-init status --long
systemctl is-active ssh NetworkManager
systemctl --failed
hostname -I
```

The cloud-init image logged a recoverable warning about a missing
`cc_netplan_nm_patch` module. Cloud-init nevertheless completed, the generated
NetworkManager connection worked, SSH was active, internet access succeeded,
and no systemd units failed.

## Firmware and OS update

Before application work, the system was fully upgraded and rebooted:

```sh
sudo apt update
sudo apt full-upgrade
sudo rpi-eeprom-update -a
sudo reboot
```

Post-reboot results:

- Pi 4 EEPROM bootloader current at 2026-05-17
- Dedicated VL805 USB-controller firmware `000138c0`, current
- Kernel `6.18.39+rpt-rpi-v8`
- no remaining package upgrades
- no reboot required
- no failed systemd units
- `vcgencmd get_throttled` returned `0x0`

Do not use `rpi-update` for this installation. It installs development
firmware outside the stable package/update path and is inappropriate for an
exhibition appliance.

## Recovery connection

```sh
ssh -F /dev/null -i ~/.ssh/id_ed25519 stags@infinite-scroll.local
```

The `-F /dev/null` form is useful when the administering workstation's normal
SSH config is unsuitable. The Pi's private keys and Wi-Fi password must never
be copied into this repository.
