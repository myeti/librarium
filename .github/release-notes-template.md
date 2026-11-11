<!-- GitHub release body, rendered by scripts/ci/release-assets.mjs. Placeholders: NOTES (the tag message), DOWNLOADS, VERSION, APPIMAGE, DEB, RULES. -->
{{NOTES}}

## Download

{{DOWNLOADS}}

Already using Librarium? The app offers this update by itself (except the Linux `.deb`, see below). `SHA256SUMS` lists the checksum of every file.

## First launch

Librarium is not signed with an Apple or Microsoft developer certificate yet, so your system warns you the first time you open it. Updates installed from inside the app are verified with Librarium's own signing key.

- **macOS:** open the `.dmg` and drag Librarium into Applications, then open it. If macOS says it cannot verify the developer, open **System Settings › Privacy & Security**, scroll down to Security and click **Open Anyway**, then confirm. After an update, macOS may ask again for access to your library folder and to removable volumes.
- **Windows:** if SmartScreen shows "Windows protected your PC", click **More info › Run anyway**.
- **Linux, AppImage:** make the file executable, then run it: `chmod +x {{APPIMAGE}} && ./{{APPIMAGE}}`. If it doesn't start, install FUSE 2 (`sudo apt install libfuse2t64` on Ubuntu). It is built on Ubuntu 24.04, so it needs a distribution at least as recent (glibc 2.39: Ubuntu 24.04, Debian 13, Fedora 40 or newer).
- **Linux, Debian/Ubuntu:** `sudo apt install ./{{DEB}}`. The `.deb` does not update itself: install the next release's `.deb` the same way.

### Kindle over USB on Linux

Kindles from 2024 onwards connect over MTP instead of showing up as a USB drive, and talking to them directly needs a permission rule for the USB device (Amazon, vendor id 1949). The `.deb` installs it for you. With the AppImage, download `{{RULES}}` from this page and install it once:

```sh
sudo install -m 644 {{RULES}} /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then unplug and reconnect the Kindle.
