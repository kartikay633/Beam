// Renders an HTML animation that defines window.render(t) and
// window.DURATION, frame by frame, into PNGs (for ffmpeg).
// Usage:
//   node media/render-video.mjs <page.html> <outDir> [fps]          all frames
//   node media/render-video.mjs <page.html> <outDir> --at 2,7.5,13   stills
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { pathToFileURL } from "node:url";

const [html, outDir, arg = "30", times] = process.argv.slice(2);
mkdirSync(outDir, { recursive: true });
const browser = await chromium.launch({ channel: "msedge" });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
await page.goto(pathToFileURL(html).href);
await page.evaluate(() => document.fonts.ready);
await page.waitForFunction(() => [...document.images].every((i) => i.complete));

const at = arg === "--at" ? times.split(",").map(Number) : null;
const fps = at ? 0 : Number(arg);
const duration = await page.evaluate(() => window.DURATION);
const frames = at ?? Array.from({ length: Math.round(duration * fps) }, (_, i) => i / fps);
for (const [i, t] of frames.entries()) {
  await page.evaluate((t) => window.render(t), t);
  const name = at ? `still-${String(t).replace(".", "_")}.png` : `f${String(i).padStart(5, "0")}.png`;
  await page.screenshot({ path: `${outDir}/${name}` });
}
await browser.close();
console.log(`${frames.length} frame(s) in ${outDir}`);
