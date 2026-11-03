package dev.beam.android.radio

import dev.beam.core.BleConnectRequest
import dev.beam.core.BleConnectResponse
import dev.beam.core.BleDirectPeer
import dev.beam.core.BleLink
import dev.beam.core.HotspotCredentials

internal data class LanRoute(val localAddress: String, val remoteAddress: String, val directRelease: (() -> Unit)? = null)
internal data class DirectRoute(val localAddress: String, val remoteAddress: String, val release: () -> Unit)
internal data class HotspotRoute(val ssid: String, val passphrase: String, val address: String?, val release: () -> Unit)
internal data class NegotiatedLink(val response: BleConnectResponse, val sendTarget: String?, val release: () -> Unit)

/** Link policy, independent of Android's radio APIs so failures can be tested. */
internal class LinkNegotiator(
    private val probeLan: suspend (List<String>, Int?) -> LanRoute?,
    private val joinDirect: suspend (BleDirectPeer) -> DirectRoute?,
    private val startHotspot: suspend () -> HotspotRoute,
    private val helloPort: Int,
) {
    suspend fun negotiate(request: BleConnectRequest, receivePort: Int, sendPort: Int? = null): NegotiatedLink {
        if (BleLink.LAN in request.links) {
            probeLan(request.lan, sendPort)?.let { route ->
                // A reachable address can belong to our retained P2P group.
                // Keep its lease and report Direct so Windows does the same.
                val link = if (route.directRelease != null) BleLink.WIFI_DIRECT else BleLink.LAN
                return NegotiatedLink(
                    BleConnectResponse(link, if (sendPort == null) "${route.localAddress}:$receivePort" else
                        if (link == BleLink.WIFI_DIRECT) route.localAddress else null, null, null),
                    sendPort?.let { "${route.remoteAddress}:$it" }, route.directRelease ?: {},
                )
            }
        }
        if (BleLink.WIFI_DIRECT in request.links) {
            request.direct?.let { peer ->
                joinDirect(peer)?.let { route ->
                    return NegotiatedLink(
                        BleConnectResponse(BleLink.WIFI_DIRECT,
                            if (sendPort == null) "${route.localAddress}:$receivePort" else route.localAddress, null, null),
                        sendPort?.let { "${route.remoteAddress}:$it" }, route.release,
                    )
                }
            }
        }
        check(BleLink.HOTSPOT in request.links) { "Couldn't establish Wi-Fi Direct, and the other device offers no fallback" }
        check(request.hotspot == null) { "Phone-to-phone transfers without shared Wi-Fi aren't supported yet" }
        val spot = startHotspot()
        return NegotiatedLink(
            BleConnectResponse(BleLink.HOTSPOT, "${spot.address ?: "0.0.0.0"}:${if (sendPort == null) receivePort else helloPort}",
                HotspotCredentials(spot.ssid, spot.passphrase), null),
            null, spot.release,
        )
    }
}
