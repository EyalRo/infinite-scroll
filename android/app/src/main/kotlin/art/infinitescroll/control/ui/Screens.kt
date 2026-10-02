package art.infinitescroll.control.ui

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import art.infinitescroll.control.InstallationViewModel
import art.infinitescroll.control.NoticeKind
import art.infinitescroll.control.UiState
import art.infinitescroll.control.ble.LinkState
import art.infinitescroll.protocol.Job
import art.infinitescroll.protocol.LibraryItem
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.delay

private fun fmtTime(unixSeconds: Double?): String =
    if (unixSeconds == null) "—" else DateFormat.getDateTimeInstance().format(Date((unixSeconds * 1000).toLong()))

private fun fmtMs(unixMs: Long): String = DateFormat.getDateTimeInstance().format(Date(unixMs))

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            content()
        }
    }
}

@Composable
private fun Line(label: String, value: String) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodyMedium)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}

// ---- connect -------------------------------------------------------------

@SuppressLint("MissingPermission") // Names are only listed after BLUETOOTH_CONNECT was granted.
@Composable
fun ConnectScreen(state: UiState, onScan: () -> Unit, onConnect: (BluetoothDevice) -> Unit, onDismissNotice: () -> Unit) {
    Scaffold { padding ->
        Column(Modifier.padding(padding).padding(16.dp).fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Infinite Scroll", style = MaterialTheme.typography.headlineMedium)
            Text("Find the installation over Bluetooth. No Wi-Fi needed.")
            state.notice?.let { Text(it.text, color = MaterialTheme.colorScheme.error); TextButton(onClick = onDismissNotice) { Text("Dismiss") } }
            when (state.link) {
                LinkState.CONNECTING -> { Text("Connecting…"); LinearProgressIndicator(Modifier.fillMaxWidth()) }
                else -> Button(onClick = onScan, enabled = !state.scanning) { Text(if (state.scanning) "Scanning…" else "Scan") }
            }
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(state.devices, key = { it.address }) { device ->
                    Card(Modifier.fillMaxWidth()) {
                        Row(Modifier.padding(16.dp).fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            Column { Text(device.name ?: "Infinite Scroll"); Text(device.address, style = MaterialTheme.typography.bodySmall) }
                            Button(onClick = { onConnect(device) }) { Text("Connect") }
                        }
                    }
                }
            }
        }
    }
}

// ---- main shell -------------------------------------------------------------

private enum class Tab(val label: String) { STATUS("Status"), LIBRARY("Library"), PRINTING("Printing"), SCHEDULE("Schedule"), PRINTER("Printer") }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ControlApp(state: UiState, vm: InstallationViewModel) {
    var tab by remember { mutableStateOf(Tab.STATUS) }
    val snackbar = remember { SnackbarHostState() }

    LaunchedEffect(state.notice) {
        val notice = state.notice ?: return@LaunchedEffect
        val prefix = when (notice.kind) { NoticeKind.ACCEPTED -> "✓ Accepted by the Pi. "; NoticeKind.COMPLETED -> "✓ Done. "; NoticeKind.ERROR -> "✗ " }
        snackbar.showSnackbar(if (notice.kind == NoticeKind.ACCEPTED) notice.text else prefix + notice.text.removePrefix("Accepted: "))
        vm.dismissNotice()
    }
    // While a job is active, poll as a backstop to the Pi's change hints.
    val anyActive = state.jobs.any { it.active }
    LaunchedEffect(anyActive) { while (anyActive) { delay(3_000); vm.refreshPrinting() } }

    Scaffold(
        topBar = { TopAppBar(title = { Text("Infinite Scroll") }, actions = { TextButton(onClick = vm::refreshAll) { Text("Refresh") }; TextButton(onClick = vm::disconnect) { Text("Disconnect") } }) },
        bottomBar = { NavigationBar { Tab.entries.forEach { t -> NavigationBarItem(selected = tab == t, onClick = { tab = t }, label = { Text(t.label) }, icon = {}) } } },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.padding(padding).padding(horizontal = 16.dp).fillMaxSize()) {
            when (tab) {
                Tab.STATUS -> StatusScreen(state, vm)
                Tab.LIBRARY -> LibraryScreen(state, vm)
                Tab.PRINTING -> PrintingScreen(state, vm)
                Tab.SCHEDULE -> ScheduleScreen(state, vm)
                Tab.PRINTER -> PrinterScreen(state, vm)
            }
        }
    }
}

// ---- status -----------------------------------------------------------------

@Composable
private fun StatusScreen(state: UiState, vm: InstallationViewModel) {
    LazyColumn {
        item {
            val status = state.status
            Section("Installation") {
                if (status == null) Text("Loading…") else {
                    Line("Printer service", if (status.services.printer) "running" else "NOT RUNNING")
                    Line("Upload service", if (status.services.uploader) "running" else "NOT RUNNING")
                    status.printerError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                    status.printer?.let { p ->
                        Line("Library items", p.catalogCount.toString())
                        p.processingCount?.let { Line("Uploads being processed", it.toString()) }
                        if (p.failedCount > 0) Text("${p.failedCount} upload(s) failed conversion.", color = MaterialTheme.colorScheme.error)
                    }
                }
            }
            Section("Clock") {
                val piMs = state.status?.clock?.unixMs
                Line("Pi time", piMs?.let(::fmtMs) ?: "—")
                Line("Phone time", fmtMs(System.currentTimeMillis()))
                Text("The Pi has no network time when offline; sync it after any power loss.", style = MaterialTheme.typography.bodySmall)
                Button(onClick = vm::syncClock) { Text("Set Pi clock from phone") }
            }
            state.stats?.let { s ->
                Section("Statistics") {
                    Line("Prints succeeded", s.printsOk.toString())
                    Line("  scheduled / queued", "${s.scheduledOk} / ${s.jobOk}")
                    Line("Prints failed", s.printsFailed.toString())
                    Line("Failed conversions", s.failedConversions.toString())
                    Line("Paper (estimated)", "%.2f m".format(s.paperMm / 1000.0))
                    Text("Counted since ${fmtTime(s.countersSince)}", style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

// ---- library ------------------------------------------------------------

@Composable
private fun LibraryScreen(state: UiState, vm: InstallationViewModel) {
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris: List<Uri> -> uris.forEach(vm::upload) }
    var printing by remember { mutableStateOf<LibraryItem?>(null) }
    var deleting by remember { mutableStateOf<LibraryItem?>(null) }
    var confirmAll by remember { mutableStateOf(false) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { picker.launch("image/*") }, enabled = state.uploadProgress == null) { Text("Upload artwork") }
            OutlinedButton(onClick = { confirmAll = true }, enabled = state.library.isNotEmpty()) { Text("Print all") }
        }
        state.uploadProgress?.let { LinearProgressIndicator(progress = { it }, modifier = Modifier.fillMaxWidth()) }
        Text("${state.library.size} item(s). PNG or JPEG, up to 20 MB.", style = MaterialTheme.typography.bodySmall)
        LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(state.library, key = { it.id }) { item ->
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp)) {
                        Text(item.originalFilename, style = MaterialTheme.typography.bodyLarge)
                        Text("Printed ${item.printCount}× · added ${fmtTime(item.addedAt)}", style = MaterialTheme.typography.bodySmall)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick = { printing = item }) { Text("Print…") }
                            TextButton(onClick = { deleting = item }) { Text("Remove") }
                        }
                    }
                }
            }
        }
    }

    printing?.let { item ->
        CopiesDialog("Print ${item.originalFilename}", onDismiss = { printing = null }) { copies -> vm.print(item, copies); printing = null }
    }
    if (confirmAll) {
        CopiesDialog("Print the entire library (${state.library.size} items), this many times each", onDismiss = { confirmAll = false }) { copies -> vm.printAll(copies); confirmAll = false }
    }
    deleting?.let { item ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Remove artwork?") },
            text = { Text("${item.originalFilename} will be removed from the library.") },
            confirmButton = { TextButton(onClick = { vm.delete(item); deleting = null }) { Text("Remove") } },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun CopiesDialog(title: String, onDismiss: () -> Unit, onConfirm: (Int) -> Unit) {
    var copies by remember { mutableIntStateOf(1) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(onClick = { copies = maxOf(1, copies - 1) }) { Text("−") }
                Text("$copies ${if (copies == 1) "copy" else "copies"}", style = MaterialTheme.typography.titleMedium)
                OutlinedButton(onClick = { copies = minOf(100, copies + 1) }) { Text("+") }
            }
        },
        confirmButton = { TextButton(onClick = { onConfirm(copies) }) { Text("Queue") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

// ---- printing -----------------------------------------------------------

@Composable
private fun JobCard(job: Job, onCancel: (() -> Unit)?) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(if (job.kind == "all") "Print all" else "Print ×${job.copies}", style = MaterialTheme.typography.bodyLarge)
            Text("${job.state} · ${job.done}/${job.total}${if (job.skipped > 0) " (${job.skipped} skipped)" else ""}", style = MaterialTheme.typography.bodySmall)
            if (job.total > 0) LinearProgressIndicator(progress = { job.done.toFloat() / job.total }, modifier = Modifier.fillMaxWidth())
            job.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            if (job.active && onCancel != null) TextButton(onClick = onCancel) { Text("Cancel job") }
        }
    }
}

@Composable
private fun PrintingScreen(state: UiState, vm: InstallationViewModel) {
    LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item {
            Section("Printer") {
                val p = state.status?.printer
                Line("State", when { p == null -> "unknown"; !p.printer.connected -> "NOT CONNECTED"; p.printer.busy -> "busy"; else -> "idle" })
                p?.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
            Text("Queue", style = MaterialTheme.typography.titleMedium)
        }
        val active = state.jobs.filter { it.active }
        if (active.isEmpty()) item { Text("Nothing queued.") }
        items(active, key = { it.id }) { JobCard(it) { vm.cancel(it) } }
        item { Text("Recent jobs", style = MaterialTheme.typography.titleMedium) }
        items(state.jobs.filter { !it.active }.asReversed(), key = { it.id }) { JobCard(it, null) }
        item { Text("Print history", style = MaterialTheme.typography.titleMedium) }
        items(state.history, key = { "${it.at}-${it.itemId}" }) { record ->
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Text("${if (record.success) "✓" else "✗"} ${record.originalFilename.ifEmpty { record.itemId }}")
                Text("${fmtTime(record.at)} · ${record.origin}${record.error?.let { " · $it" } ?: ""}", style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

// ---- schedule -----------------------------------------------------------

@Composable
private fun ScheduleScreen(state: UiState, vm: InstallationViewModel) {
    val schedule = state.schedule
    if (schedule == null) { Text("Loading…"); return }
    var min by remember(schedule.minMinutes) { mutableStateOf(schedule.minMinutes.toString()) }
    var max by remember(schedule.maxMinutes) { mutableStateOf(schedule.maxMinutes.toString()) }
    var ordering by remember(schedule.ordering) { mutableStateOf(schedule.ordering) }

    LazyColumn {
        item {
            Section("Autoprint") {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text(if (schedule.enabled) "Enabled" else "Disabled")
                    Switch(checked = schedule.enabled, onCheckedChange = vm::setAutoprint)
                }
                Line("Next print", if (schedule.enabled) fmtTime(schedule.nextPrintAt) else "—")
                schedule.lastError?.let { Text("Last error: $it", color = MaterialTheme.colorScheme.error) }
            }
            Section("Timing") {
                OutlinedTextField(min, { min = it }, label = { Text("Minimum minutes") }, singleLine = true)
                OutlinedTextField(max, { max = it }, label = { Text("Maximum minutes") }, singleLine = true)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("sequential", "random").forEach { o -> FilterChip(selected = ordering == o, onClick = { ordering = o }, label = { Text(o) }) }
                }
                Button(onClick = {
                    val lo = min.toDoubleOrNull(); val hi = max.toDoubleOrNull()
                    if (lo != null && hi != null) vm.setSchedule(lo, hi, ordering)
                }) { Text("Save timing") }
                Text("Saving re-arms the timer. Maximum is 7 days (10080 minutes).", style = MaterialTheme.typography.bodySmall)
            }
            Section("Preview — next picks") {
                val preview = state.preview
                if (preview == null || preview.picks.isEmpty()) Text("The library is empty — nothing to preview.") else {
                    if (!preview.exact) Text("Random order: this is one possible outcome.", style = MaterialTheme.typography.bodySmall)
                    preview.picks.forEachIndexed { i, item -> Text("${i + 1}. ${item.originalFilename}") }
                }
            }
        }
    }
}

// ---- printer settings ----------------------------------------------------------

@Composable
private fun PrinterScreen(state: UiState, vm: InstallationViewModel) {
    LazyColumn {
        item {
            Section("Printer status") {
                val p = state.status?.printer
                Line("Connection", if (p == null) "unknown" else if (p.printer.connected) "connected" else "NOT CONNECTED")
                Line("Activity", if (p == null) "unknown" else if (p.printer.busy) "busy" else "idle")
                p?.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
            // Rendered strictly from what the Pi advertises; settings appear only once verified on the printer.
            val settings = state.capabilities?.settings.orEmpty().filter { it.writable }
            Section("Print settings") {
                if (settings.isEmpty()) Text("No adjustable printer settings have been verified on this printer yet.")
                settings.forEach { cap ->
                    var value by remember(cap.key) { mutableFloatStateOf(cap.min.toFloat()) }
                    Text("${cap.label}: ${value.toLong()}")
                    val steps = ((cap.max - cap.min) / cap.step - 1).toInt().coerceAtLeast(0)
                    Slider(value = value, onValueChange = { value = it }, valueRange = cap.min.toFloat()..cap.max.toFloat(), steps = steps)
                    Button(onClick = { vm.setPrinterSetting(cap.key, value.toLong()) }) { Text("Apply") }
                }
            }
        }
    }
}
