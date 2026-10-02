package art.infinitescroll.control.ui

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import art.infinitescroll.control.InstallationViewModel
import art.infinitescroll.control.NoticeKind
import art.infinitescroll.control.UiState
import art.infinitescroll.control.ble.LinkState
import art.infinitescroll.protocol.Job
import art.infinitescroll.protocol.LibraryItem
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.delay

private fun fmtMinutes(minutes: Int): String = "%02d:%02d".format(minutes / 60, minutes % 60)

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
                LaunchedEffect(item.id) { vm.loadThumbnail(item.id) }
                Card(Modifier.fillMaxWidth()) {
                    Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        val thumbnail = state.thumbnails[item.id]
                        if (thumbnail != null) {
                            Image(
                                bitmap = thumbnail,
                                contentDescription = "Preview of ${item.originalFilename}",
                                modifier = Modifier.width(72.dp).aspectRatio(thumbnail.width.toFloat() / thumbnail.height.toFloat()),
                                contentScale = ContentScale.Fit,
                            )
                        } else {
                            Box(Modifier.width(72.dp).aspectRatio(0.67f)) {} // keeps rows from jumping while it loads
                        }
                        Column {
                            Text(item.originalFilename, style = MaterialTheme.typography.bodyLarge)
                            Text("Printed ${item.printCount}× · added ${fmtTime(item.addedAt)}", style = MaterialTheme.typography.bodySmall)
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                // One tap queues exactly one print; the Pi owns the job from there.
                                FilledTonalIconButton(
                                    onClick = { vm.print(item, 1) },
                                    modifier = Modifier.semantics { contentDescription = "Print ${item.originalFilename}" },
                                ) { Text("🖨️", fontSize = 20.sp) }
                                FilledTonalIconButton(
                                    onClick = { deleting = item },
                                    modifier = Modifier.semantics { contentDescription = "Remove ${item.originalFilename}" },
                                ) { Text("🗑️", fontSize = 20.sp) }
                            }
                        }
                    }
                }
            }
        }
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

/** The art-show button: the whole library, this many times each. */
private const val SHOW_COPIES = 10

@Composable
private fun PrintingScreen(state: UiState, vm: InstallationViewModel) {
    var confirm by remember { mutableStateOf(false) }
    val items = state.library.size
    val busy = state.jobs.any { it.active }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item {
            Button(
                onClick = { confirm = true },
                enabled = items > 0 && !busy,
                modifier = Modifier.fillMaxWidth().height(120.dp).padding(vertical = 6.dp),
            ) { Text("PRINT!", style = MaterialTheme.typography.displayMedium) }
            Text(
                when {
                    items == 0 -> "The library is empty."
                    busy -> "A print job is already running."
                    else -> "Prints the entire library $SHOW_COPIES times ($items items, ${items * SHOW_COPIES} prints)."
                },
                style = MaterialTheme.typography.bodySmall,
            )
        }
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

    if (confirm) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text("Print the whole library?") },
            text = { Text("$items items \u00d7 $SHOW_COPIES = ${items * SHOW_COPIES} prints. The Pi will keep printing even if you disconnect.") },
            confirmButton = { TextButton(onClick = { vm.printAll(SHOW_COPIES); confirm = false }) { Text("Print!") } },
            dismissButton = { TextButton(onClick = { confirm = false }) { Text("Cancel") } },
        )
    }
}

// ---- schedule -----------------------------------------------------------

private const val MIN_DELAY_MINUTES = 1f
private const val MAX_DELAY_MINUTES = 60f

@Composable
private fun ScheduleScreen(state: UiState, vm: InstallationViewModel) {
    val schedule = state.schedule
    if (schedule == null) { Text("Loading…"); return }
    var lo by remember(schedule.minMinutes) { mutableFloatStateOf(schedule.minMinutes.toFloat().coerceIn(MIN_DELAY_MINUTES, MAX_DELAY_MINUTES)) }
    var hi by remember(schedule.maxMinutes) { mutableFloatStateOf(schedule.maxMinutes.toFloat().coerceIn(MIN_DELAY_MINUTES, MAX_DELAY_MINUTES)) }
    var ordering by remember(schedule.ordering) { mutableStateOf(schedule.ordering) }
    var windowOn by remember(schedule.windowEnabled) { mutableStateOf(schedule.windowEnabled) }
    var from by remember(schedule.windowStart) { mutableFloatStateOf(schedule.windowStart / 15f) }
    var until by remember(schedule.windowEnd) { mutableFloatStateOf(schedule.windowEnd / 15f) }
    val fromMinute = from.roundToInt() * 15
    val untilMinute = until.roundToInt() * 15

    LazyColumn {
        item {
            Section("Autoprint") {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text(if (schedule.enabled) "Enabled" else "Disabled")
                    Switch(checked = schedule.enabled, onCheckedChange = vm::setAutoprint)
                }
                Line("Next print", if (schedule.enabled) fmtTime(schedule.nextPrintAt) else "—")
                schedule.lastError?.let { Text("Last error: $it", color = MaterialTheme.colorScheme.error) }

                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("Only print during these hours")
                    Switch(checked = windowOn, onCheckedChange = { windowOn = it })
                }
                if (windowOn) {
                    Text("From ${fmtMinutes(fromMinute)}")
                    Slider(value = from, onValueChange = { from = it }, valueRange = 0f..95f, steps = 94)
                    Text("Until ${fmtMinutes(untilMinute)}")
                    Slider(value = until, onValueChange = { until = it }, valueRange = 0f..95f, steps = 94)
                }
                Button(onClick = { vm.setPrintWindow(windowOn, fromMinute, untilMinute) }, enabled = !windowOn || fromMinute != untilMinute) { Text("Save hours") }
                Text(
                    "Pi local time. A print that falls due outside these hours waits for the window to open; " +
                        "a window like 22:00 to 02:00 runs overnight. Printing on demand ignores it.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Section("Randomness") {
                Text("Wait between ${lo.roundToInt()} and ${hi.roundToInt()} minutes between prints")
                Text("At least: ${lo.roundToInt()} min")
                Slider(
                    value = lo,
                    onValueChange = { lo = it; if (hi < it) hi = it },
                    valueRange = MIN_DELAY_MINUTES..MAX_DELAY_MINUTES,
                    steps = (MAX_DELAY_MINUTES - MIN_DELAY_MINUTES).toInt() - 1,
                )
                Text("At most: ${hi.roundToInt()} min")
                Slider(
                    value = hi,
                    onValueChange = { hi = it; if (lo > it) lo = it },
                    valueRange = MIN_DELAY_MINUTES..MAX_DELAY_MINUTES,
                    steps = (MAX_DELAY_MINUTES - MIN_DELAY_MINUTES).toInt() - 1,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("sequential", "random").forEach { o -> FilterChip(selected = ordering == o, onClick = { ordering = o }, label = { Text(o) }) }
                }
                Button(onClick = { vm.setSchedule(lo.roundToInt().toDouble(), hi.roundToInt().toDouble(), ordering) }) { Text("Save timing") }
                Text("Saving re-arms the timer. Each wait is a random time between the two values.", style = MaterialTheme.typography.bodySmall)
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
                state.printerSettingsError?.let { Text("Could not read the printer: $it", color = MaterialTheme.colorScheme.error) }
                settings.forEach { cap ->
                    val current = state.printerValues[cap.key]
                    var value by remember(cap.key, current) { mutableFloatStateOf((current ?: cap.min).toFloat()) }
                    Text("${cap.label}: ${value.toLong()}" + (current?.let { " (printer is at $it)" } ?: ""))
                    val steps = ((cap.max - cap.min) / cap.step - 1).toInt().coerceAtLeast(0)
                    Slider(value = value, onValueChange = { value = it }, valueRange = cap.min.toFloat()..cap.max.toFloat(), steps = steps)
                    if (cap.key == "darkness") Text("Higher is darker: 0 is the lightest print, 30 the darkest.", style = MaterialTheme.typography.bodySmall)
                    Button(onClick = { vm.setPrinterSetting(cap.key, value.toLong()) }, enabled = current != value.toLong()) { Text("Apply") }
                }
                state.capabilities?.actions.orEmpty().firstOrNull { it.key == "print_config" }?.let { action ->
                    OutlinedButton(onClick = vm::printConfig) { Text(action.label) }
                    Text("The printer prints its own configuration label.", style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}
