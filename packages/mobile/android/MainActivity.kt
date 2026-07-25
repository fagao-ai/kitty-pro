package dev.dioxus.main

import android.content.Intent
import android.os.Bundle
import com.kitty.pro.KittyVpnBridge

class MainActivity : WryActivity() {
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        KittyVpnBridge.onActivityResult(this, requestCode, resultCode)
    }
}
