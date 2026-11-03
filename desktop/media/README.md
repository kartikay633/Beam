# Store media

How the Microsoft Store screenshots and the trailer are made, from the
real desktop UI in demo mode (no real devices or files):

1. `npm run dev`, then `node media/capture.mjs <out> [qr.svg]` saves light
   and dark captures (2560x1600) of Share, Your devices, Settings,
   Activity and the incoming-files window.
2. Copy them, a phone screenshot (`phone-home.png`, status and navigation
   bars cropped) and `src-tauri/icons/128x128@2x.png` (`logo.png`) into
   `assets/` next to `promo.html`.
3. `node media/render-video.mjs promo.html frames 30` renders every frame
   (`--at 2,7.5` renders stills to check a layout).
4. Encode: `ffmpeg -framerate 30 -i frames/f%05d.png -c:v libx264 -crf 16
   -pix_fmt yuv420p -movflags +faststart trailer.mp4`.
