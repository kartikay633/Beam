package dev.beam.android.service

import android.graphics.drawable.Icon
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dev.beam.android.BeamApp
import dev.beam.android.R
import dev.beam.core.Visibility

/**
 * Quick Settings tile (SPEC.md 8): one tap makes the phone visible to
 * everyone nearby for 10 minutes; another goes back to trusted devices only.
 */
class VisibilityTileService : TileService() {
    private val repository get() = (application as BeamApp).repository

    override fun onStartListening() = render()

    override fun onClick() {
        val next = if (repository.state.value.visibility == Visibility.EVERYONE) Visibility.TRUSTED_ONLY else Visibility.EVERYONE
        repository.setVisibility(next)
        render()
    }

    private fun render() {
        val tile = qsTile ?: return
        val visibility = repository.state.value.visibility
        tile.icon = Icon.createWithResource(this, R.drawable.ic_notification)
        tile.label = "Beam"
        tile.state = if (visibility == Visibility.EVERYONE) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.subtitle = when (visibility) {
            Visibility.EVERYONE -> "Everyone, 10 min"
            Visibility.TRUSTED_ONLY -> "Trusted only"
            Visibility.OFF -> "Hidden"
        }
        tile.updateTile()
    }
}
