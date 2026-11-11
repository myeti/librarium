/**
 * The app's few sound effects, played at the volume chosen in Options
 * (`settings.soundVolume`; 0 mutes them). Playback is best effort: a sound
 * the webview refuses to play (autoplay policy, no audio device) is skipped
 * silently.
 */
import introSrc from '@/assets/sounds/intro.mp3'
import newUserSrc from '@/assets/sounds/new-user.mp3'
import transferDoneSrc from '@/assets/sounds/transfer-done.mp3'
import { useSettingsStore } from '@/stores/settings'

function play(src: string) {
    const volume = useSettingsStore().soundVolume
    if (volume === 0) {return}
    const audio = new Audio(src)
    audio.volume = volume
    audio.play().catch(() => {})
}

/** At startup, once the window is ready. */
export function playIntro() {
    play(introSrc)
}

/** After a user was created. */
export function playNewUser() {
    play(newUserSrc)
}

/** When every queued transfer has finished. */
export function playTransferComplete() {
    play(transferDoneSrc)
}
