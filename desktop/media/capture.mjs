// Captures the desktop UI (demo mode, no real devices) for the Store
// listing and the promo video. Needs the Vite dev server on :1420
// (`npm run dev`). Usage: node media/capture.mjs <outDir> <qr.svg>
import { chromium } from "@playwright/test";
import { readFileSync, mkdirSync } from "node:fs";

const [outDir = "media-out", qrPath] = process.argv.slice(2);
mkdirSync(outDir, { recursive: true });
const qrSvg = qrPath ? readFileSync(qrPath, "utf8") : null;

// Extra demo state, appended to the demo API module (where `demo` lives).
const showcase = `
const t0 = Date.now();
demo.settings.askedBackground = true;
demo.settings.startWithWindows = true;
demo.settings.pinnedFolders = ["C:\\\\Users\\\\you\\\\Pictures\\\\Trip photos"];
demo.transfers = [
  { id: "s1", direction: "send", peerName: "Kartikay's Pixel", fileNames: ["Project report.pdf", "Slides.pptx"], fileCount: 2,
    totalBytes: 48_200_000, doneBytes: 31_000_000, state: "active", detail: null, speedBps: 62_000_000, etaSecs: 1,
    verificationCode: null, folder: null, startedMs: t0 - 2_000 },
  { id: "r1", direction: "receive", peerName: "Kartikay's Pixel", fileNames: ["IMG_20261103_1842.jpg", "IMG_20261103_1843.jpg", "VID_20261103_1850.mp4"], fileCount: 3,
    totalBytes: 412_000_000, doneBytes: 412_000_000, state: "done", detail: null, speedBps: 0, etaSecs: null,
    verificationCode: null, folder: "C:\\\\Users\\\\you\\\\Desktop\\\\Trip 2026", startedMs: t0 - 600_000 },
  { id: "r2", direction: "receive", peerName: "Kartikay's Pixel", fileNames: ["Boarding pass.pdf"], fileCount: 1,
    totalBytes: 860_000, doneBytes: 860_000, state: "done", detail: null, speedBps: 0, etaSecs: null,
    verificationCode: null, folder: "C:\\\\Users\\\\you\\\\Downloads", startedMs: t0 - 3_600_000 },
];
${qrSvg ? `api.pairing = async () => ({ uri: "beam://pair", svg: ${JSON.stringify(qrSvg)}, deviceId: demo.me.deviceId, addresses: [], validForSecs: 300 });` : ""}
`;

// The installed Edge, like the Playwright tests (no browser download).
const browser = await chromium.launch({ channel: "msedge" });
for (const scheme of ["light", "dark"]) {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    colorScheme: scheme,
    reducedMotion: "reduce",
  });
  const page = await context.newPage();
  await page.route("**/src/api.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()) + showcase });
  });
  const shot = async (name) => {
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${outDir}/${name}-${scheme}.png` });
  };

  await page.goto("http://localhost:1420/");
  await page.getByRole("heading", { name: "Share", exact: true }).waitFor();
  await shot("share");

  await page.getByRole("button", { name: "Your devices" }).click();
  await shot("devices");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByText("Your trusted devices can also save to").scrollIntoViewIfNeeded();
  await shot("settings");

  await page.getByRole("button", { name: "Activity" }).click();
  await shot("activity");

  await page.setViewportSize({ width: 460, height: 560 });
  await page.goto("http://localhost:1420/#/offer/demo");
  await page.reload();
  await page.getByRole("button", { name: /accept/i }).first().waitFor();
  await shot("offer");
  await context.close();
}
await browser.close();
console.log("captured to", outDir);
