const uploaderOrigin = document.querySelector('meta[name="uploader-origin"]').content;
const printerOrigin = document.querySelector('meta[name="printer-origin"]').content;

const catalogList = document.getElementById("catalog-list");
const uploadForm = document.getElementById("upload-form");
const fileInput = document.getElementById("file-input");
const uploadStatus = document.getElementById("upload-status");
const paperStrip = document.getElementById("paper-strip");

async function fetchCatalog() {
  const response = await fetch(`${printerOrigin}/catalog`, { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load catalog: ${response.status}`);
  return (await response.json()).catalog;
}

async function fetchStatus() {
  const response = await fetch(`${printerOrigin}/status`, { credentials: "include" });
  if (!response.ok) throw new Error(`failed to load status: ${response.status}`);
  return response.json();
}

async function uploadFile(file) {
  const response = await fetch(`${uploaderOrigin}/uploads`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": file.type },
    body: file,
  });
  if (!response.ok) throw new Error(`upload failed for ${file.name}: ${response.status}`);
  return response.json();
}

async function removeItem(id) {
  const response = await fetch(`${printerOrigin}/catalog/${id}`, { method: "DELETE", credentials: "include" });
  if (!response.ok) throw new Error(`failed to remove ${id}: ${response.status}`);
}

function renderCatalog(items) {
  catalogList.innerHTML = "";
  for (const item of items) {
    const li = document.createElement("li");
    const label = document.createElement("span");
    label.textContent = `${item.original_filename} — printed ${item.print_count}×`;
    const removeButton = document.createElement("button");
    removeButton.textContent = "Remove";
    removeButton.addEventListener("click", async () => {
      await removeItem(item.id);
      await refreshCatalog();
    });
    li.append(label, removeButton);
    catalogList.append(li);
  }
}

/// Simulates 3 scheduler picks client-side, honoring the real ordering
/// setting, without ever calling the printer. Sequential mirrors the
/// printer's own choose_item exactly (advance past last_item_id, wrap);
/// random draws uniformly, avoiding an immediate repeat when possible.
function simulateLoops(items, ordering, lastItemId, count) {
  if (items.length === 0) return [];
  const picks = [];
  let last = lastItemId;
  for (let i = 0; i < count; i++) {
    let next;
    if (ordering === "sequential") {
      const lastIndex = items.findIndex((item) => item.id === last);
      next = items[(lastIndex + 1) % items.length];
    } else {
      const candidates = items.filter((item) => item.id !== last);
      const pool = candidates.length > 0 ? candidates : items;
      next = pool[Math.floor(Math.random() * pool.length)];
    }
    picks.push(next);
    last = next.id;
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
    loop.textContent = pick.original_filename;
    paperStrip.append(loop);
  }
}

async function refreshCatalog() {
  const [items, status] = await Promise.all([fetchCatalog(), fetchStatus()]);
  renderCatalog(items);
  renderPreview(items, status);
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

refreshCatalog().catch((error) => {
  uploadStatus.textContent = error.message;
});
