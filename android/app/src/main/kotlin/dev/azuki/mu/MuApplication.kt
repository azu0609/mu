package dev.azuki.mu

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import dev.azuki.mu.core.Core
import dev.azuki.mu.design.Pictograms
import kotlin.concurrent.thread

class MuApplication : Application() {
    val core: Core by lazy { Core(filesDir.path) }

    override fun onCreate() {
        super.onCreate()
        thread { Pictograms.all }
        getSystemService(ConnectivityManager::class.java)
            .registerDefaultNetworkCallback(
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) = core.nudge()
                }
            )
    }
}
