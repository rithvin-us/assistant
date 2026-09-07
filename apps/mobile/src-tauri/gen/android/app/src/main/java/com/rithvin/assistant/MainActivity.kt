package com.rithvin.assistant

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    try {
      WebView(this).clearCache(true)
    } catch (_: Exception) {}
    super.onCreate(savedInstanceState)
  }
}
