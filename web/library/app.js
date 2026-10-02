const catalogList = document.getElementById("catalog-list");
const uploadForm = document.getElementById("upload-form");
const fileInput = document.getElementById("file-input");
const uploadStatus = document.getElementById("upload-status");
const paperStrip = document.getElementById("paper-strip");
const timerForm = document.getElementById("timer-form");
const timerEnabled = document.getElementById("timer-enabled");
const timerMin = document.getElementById("timer-min");
const timerMax = document.getElementById("timer-max");
const timerStatus = document.getElementById("timer-status");

async function fetchCatalog() {
  const response = await fetch("/catalog", { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load catalog: ${response.status}`);
  return (await response.json()).catalog;
}

async function fetchStatus() {
  const response = await fetch("/status", { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load status: ${response.status}`);
  return response.json();
}

async function uploadFile(file) {
  // Same-origin now: printer proxies this to the uploader service
  // internally (see upload_proxy.rs) instead of the browser calling
  // uploader's own separate origin directly, which needed its own
  // Cloudflare Access login session and kept breaking real uploads
  // whenever that session expired or never existed in a given browser.
  const response = await fetch("/uploads", {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": file.type },
    body: file,
  });
  if (!response.ok) throw new Error(`upload failed for ${file.name}: ${response.status}`);
  return response.json();
}

async function removeItem(id) {
  const response = await fetch(`/catalog/${id}`, { method: "DELETE", credentials: "include" });
  if (!response.ok) throw new Error(`failed to remove ${id}: ${response.status}`);
}

async function printNow(id) {
  // Deliberately separate from the autoprint scheduler -- see the server's
  // manual_print_response doc comment: this never touches Settings, so it
  // doesn't consume a slot in or reorder the current shuffle/sequential pass.
  const response = await fetch(`/catalog/${id}/print`, { method: "POST", credentials: "include" });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `print failed: ${response.status}`);
  return body;
}

async function saveAutoprint(enabled, minMinutes, maxMinutes) {
  // Same-origin (printer serves this page too), so unlike uploadFile() there's
  // no CORS preflight to dodge -- a normal application/json Content-Type is fine.
  const response = await fetch("/autoprint", {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ enabled, min_minutes: minMinutes, max_minutes: maxMinutes }),
  });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `failed to save timer settings: ${response.status}`);
  return body.autoprint;
}

function renderTimer(autoprint) {
  timerEnabled.checked = autoprint.enabled;
  timerMin.value = autoprint.min_minutes;
  timerMax.value = autoprint.max_minutes;
}

function renderCatalog(items) {
  catalogList.innerHTML = "";
  for (const item of items) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = `${item.original_filename} — printed ${item.print_count}×`;

    const printButton = document.createElement("button");
    printButton.className = "print-button";
    printButton.textContent = "Print now";
    printButton.addEventListener("click", async () => {
      printButton.disabled = true;
      uploadStatus.textContent = `Printing ${item.original_filename}…`;
      try {
        await printNow(item.id);
        uploadStatus.textContent = "Printed.";
        await refreshCatalog();
      } catch (error) {
        uploadStatus.textContent = error.message;
      } finally {
        printButton.disabled = false;
      }
    });

    const removeButton = document.createElement("button");
    removeButton.className = "remove-button";
    removeButton.textContent = "Remove";
    removeButton.addEventListener("click", async () => {
      try {
        await removeItem(item.id);
        await refreshCatalog();
      } catch (error) {
        uploadStatus.textContent = error.message;
      }
    });

    const actions = document.createElement("div");
    actions.className = "catalog-actions";
    actions.append(printButton, removeButton);
    li.append(label, actions);
    catalogList.append(li);
  }
}

function shuffledCopy(items) {
  const copy = items.slice();
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy;
}

/// Simulates `loopCount` full passes over the library client-side, honoring
/// the real ordering setting, without ever calling the printer. "sequential"
/// mirrors the printer's own choose_item exactly: each pass continues in
/// list order from just past last_item_id, wrapping. "random" mirrors the
/// printer's shuffle-a-full-pass-then-reshuffle behavior: each pass is its
/// own independent shuffled permutation of the whole library (this can't
/// predict the printer's actual future draws -- randomness -- it just
/// shows the same *shape* of behavior).
function simulateLoops(items, ordering, lastItemId, loopCount) {
  if (items.length === 0) return [];
  const picks = [];
  if (ordering === "sequential") {
    const lastIndex = items.findIndex((item) => item.id === lastItemId);
    const start = lastIndex === -1 ? 0 : (lastIndex + 1) % items.length;
    for (let i = 0; i < items.length * loopCount; i++) {
      picks.push(items[(start + i) % items.length]);
    }
  } else {
    for (let pass = 0; pass < loopCount; pass++) {
      picks.push(...shuffledCopy(items));
    }
  }
  return picks;
}

function renderPreview(items, status) {
  paperStrip.innerHTML = "";
  const picks = simulateLoops(items, status.autoprint.ordering, status.autoprint.last_item_id ?? null, 3);
  if (picks.length === 0) {
    paperStrip.textContent = "The library is empty — nothing to preview yet.";
    return;
  }
  for (const pick of picks) {
    const loop = document.createElement("div");
    loop.className = "loop";
    const img = document.createElement("img");
    img.src = `/catalog/${pick.id}/preview.png`;
    img.alt = pick.original_filename;
    loop.append(img);
    paperStrip.append(loop);
  }
}

async function refreshCatalog() {
  const [items, status] = await Promise.all([fetchCatalog(), fetchStatus()]);
  renderCatalog(items);
  renderPreview(items, status);
  renderTimer(status.autoprint);
  if (status.failed_count > 0) {
    uploadStatus.textContent += ` (${status.failed_count} upload(s) failed conversion — check the watcher)`;
  }
}

uploadForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const files = Array.from(fileInput.files);
  if (files.length === 0) return;
  uploadStatus.textContent = `Uploading ${files.length} file(s)…`;
  try {
    for (const file of files) {
      await uploadFile(file);
    }
    uploadStatus.textContent = "Done.";
    fileInput.value = "";
    await refreshCatalog();
  } catch (error) {
    uploadStatus.textContent = error.message;
  }
});

timerForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  timerStatus.textContent = "Saving…";
  try {
    const autoprint = await saveAutoprint(timerEnabled.checked, timerMin.valueAsNumber, timerMax.valueAsNumber);
    renderTimer(autoprint);
    timerStatus.textContent = "Saved.";
  } catch (error) {
    timerStatus.textContent = error.message;
  }
});

refreshCatalog().catch((error) => {
  uploadStatus.textContent = error.message;
});
