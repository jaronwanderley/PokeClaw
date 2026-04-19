package io.agents.pokeclaw

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.util.Log
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
    companion object {
        private const val TAG = "MainActivity"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        requestAllFilesAccessIfNeeded()
    }

    override fun onResume() {
        super.onResume()
        // Re-check when user returns from settings
        requestAllFilesAccessIfNeeded()
    }

    private fun requestAllFilesAccessIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            if (!Environment.isExternalStorageManager()) {
                Log.i(TAG, "All Files Access not granted — opening settings")
                try {
                    val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
                    intent.data = Uri.parse("package:$packageName")
                    startActivity(intent)
                } catch (e: Exception) {
                    // Fallback for OEM-modified settings screens
                    Log.w(TAG, "Specific intent failed, trying generic: ${e.message}")
                    try {
                        val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                        startActivity(intent)
                    } catch (e2: Exception) {
                        Log.e(TAG, "Cannot open All Files Access settings: ${e2.message}")
                    }
                }
            } else {
                Log.i(TAG, "All Files Access granted — POSIX mmap available")
            }
        }
    }
}
