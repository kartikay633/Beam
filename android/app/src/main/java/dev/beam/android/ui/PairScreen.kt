package dev.beam.android.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors
import kotlinx.coroutines.launch

private sealed interface PairStep {
    data object Scanning : PairStep
    data class Pairing(val code: String) : PairStep
    data class Done(val name: String) : PairStep
    data class Failed(val reason: String) : PairStep
}

/** Scan the desktop's "Pair a phone" QR code; both sides then trust each other. */
@Composable
fun PairScreen(vm: BeamViewModel, onClose: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var step by remember { mutableStateOf<PairStep>(PairStep.Scanning) }
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    var denied by remember { mutableStateOf(false) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        granted = it
        denied = !it
    }
    var pasted by remember { mutableStateOf("") }

    fun pair(code: String) {
        if (step is PairStep.Pairing) return
        step = PairStep.Pairing(code)
        scope.launch {
            step = vm.pair(code).fold({ PairStep.Done(it.name) }, { PairStep.Failed(it.message ?: "pairing failed") })
        }
    }

    Column(
        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Pair with a laptop", style = MaterialTheme.typography.headlineSmall)
        Text("On the laptop, open Beam and choose \"Pair a phone\", then point the camera at the QR code.")

        when (val s = step) {
            PairStep.Scanning -> when {
                granted -> QrScanner(Modifier.fillMaxWidth().aspectRatio(1f)) { if (it.startsWith("beam://pair")) pair(it) }
                else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        if (denied) "Without the camera, paste the pairing link instead (the laptop can copy it)."
                        else "Beam needs the camera only to read the pairing QR code. Nothing is recorded.",
                    )
                    if (denied) {
                        TextButton(onClick = {
                            context.startActivity(
                                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
                            )
                        }) { Text("Open settings") }
                    } else {
                        Button(onClick = { request.launch(Manifest.permission.CAMERA) }) { Text("Allow camera") }
                    }
                }
            }
            is PairStep.Pairing -> Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator()
                Text("  Connecting to the laptop…")
            }
            is PairStep.Done -> {
                Text("Paired with ${s.name}. You can now send to each other without being visible to everyone.")
                Button(onClick = onClose) { Text("Done") }
            }
            is PairStep.Failed -> {
                Text("Pairing failed: ${s.reason}", color = MaterialTheme.colorScheme.error)
                Text("Make sure both devices are on the same Wi-Fi, and that the code is less than 5 minutes old.")
                Button(onClick = { step = PairStep.Scanning }) { Text("Try again") }
            }
        }

        if (step is PairStep.Scanning) {
            OutlinedTextField(
                value = pasted,
                onValueChange = { pasted = it },
                label = { Text("Or paste the pairing link") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            TextButton(onClick = { pair(pasted.trim()) }, enabled = pasted.trim().startsWith("beam://pair")) { Text("Pair") }
        }
        TextButton(onClick = onClose) { Text("Close") }
    }
}

/** Camera preview that reports every QR code it reads. */
@OptIn(ExperimentalGetImage::class) // ImageProxy.image, the frame ML Kit reads.
@Composable
private fun QrScanner(modifier: Modifier, onCode: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val previewView = remember { PreviewView(context) }
    val executor = remember { Executors.newSingleThreadExecutor() }
    val scanner = remember {
        BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
    }

    DisposableEffect(lifecycleOwner) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        providerFuture.addListener({
            val p = providerFuture.get()
            provider = p
            val preview = Preview.Builder().build().also { it.surfaceProvider = previewView.surfaceProvider }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            analysis.setAnalyzer(executor) { proxy ->
                val image = proxy.image
                if (image == null) {
                    proxy.close()
                    return@setAnalyzer
                }
                scanner.process(InputImage.fromMediaImage(image, proxy.imageInfo.rotationDegrees))
                    .addOnSuccessListener { codes -> codes.firstNotNullOfOrNull { it.rawValue }?.let(onCode) }
                    .addOnCompleteListener { proxy.close() }
            }
            p.unbindAll()
            p.bindToLifecycle(lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        }, ContextCompat.getMainExecutor(context))
        onDispose {
            provider?.unbindAll()
            scanner.close()
            executor.shutdown()
        }
    }
    Box(modifier) { AndroidView({ previewView }, Modifier.fillMaxWidth().aspectRatio(1f)) }
}
