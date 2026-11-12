package com.kitty.pro

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * Shows the system's "allow Kitty Pro to set up a VPN connection?" dialog and
 * waits for it, then starts, or refuses to start, the tunnel.
 *
 * This exists because that dialog is a result an activity has to receive, and the
 * activity the user is looking at is NSH's `com.longbridge.nsh.AppActivity`, which
 * is `final` and does not forward `onActivityResult`. A prebuilt runtime cannot be
 * subclassed, so the request moves to a window of ours that draws nothing: it
 * launches the system intent, and finishes before it can be seen.
 *
 * The config is carried through here rather than kept in [KittyVpnBridge] waiting
 * for a callback. It is a process-local value either way, but holding it in the
 * intent means a process that Android killed and restarted while the dialog was up
 * resumes with the config it was asked to use instead of a null.
 */
class KittyVpnPermissionActivity : Activity() {
    companion object {
        const val APPROVAL_EXTRA = "approval"
        const val CONFIG_EXTRA = "config"
        private const val REQUEST = 23141
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val approval = intent.getParcelableExtra<Intent>(APPROVAL_EXTRA)
        if (approval == null) {
            // Reaching here means the caller forgot the extra rather than that the
            // user refused, and saying so is the difference between a bug report
            // and a mystery.
            KittyVpnBridge.updateState("error: VPN authorization request was lost")
            finish()
            return
        }
        startActivityForResult(approval, REQUEST)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        val config = intent.getStringExtra(CONFIG_EXTRA)
        if (requestCode != REQUEST || resultCode != RESULT_OK || config.isNullOrBlank()) {
            KittyVpnBridge.updateState("error: Android VPN authorization was denied")
        } else {
            KittyVpnBridge.launchService(applicationContext, config)
        }
        finish()
    }

    override fun onBackPressed() {
        // Back at a permission dialog is a refusal. Left to the default this window
        // would close with the core still told it was starting.
        KittyVpnBridge.updateState("error: Android VPN authorization was denied")
        finish()
    }
}
