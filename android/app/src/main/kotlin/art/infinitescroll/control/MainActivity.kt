package art.infinitescroll.control

import android.Manifest
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import art.infinitescroll.control.ble.LinkState
import art.infinitescroll.control.ui.ConnectScreen
import art.infinitescroll.control.ui.ControlApp

class MainActivity : ComponentActivity() {
    private val viewModel: InstallationViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MaterialTheme { Root(viewModel) } }
    }
}

@Composable
private fun Root(viewModel: InstallationViewModel) {
    val state by viewModel.state.collectAsState()
    val permissions = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted.values.all { it }) viewModel.startScan()
    }
    if (state.link == LinkState.CONNECTED) {
        ControlApp(state, viewModel)
    } else {
        ConnectScreen(
            state = state,
            onScan = { permissions.launch(arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)) },
            onDismissNotice = viewModel::dismissNotice,
        )
    }
}
