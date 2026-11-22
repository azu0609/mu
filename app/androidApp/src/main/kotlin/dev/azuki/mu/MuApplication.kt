package dev.azuki.mu

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import dev.azuki.mu.core.Core

class MuApplication : Application() {
    val core: Core by lazy { Core(filesDir.path) }

    override fun onCreate() {
        super.onCreate()
        getSystemService(ConnectivityManager::class.java)
            .registerDefaultNetworkCallback(
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) = core.nudge()
                }
            )
    }
}
