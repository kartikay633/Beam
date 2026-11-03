import { expect, test } from "@playwright/test";

test("all main screens and pairing are reachable", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Share", exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/share.png", fullPage: true });
  await page.getByRole("button", { name: "Your devices", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Pair a phone", exact: true })).toBeVisible();
  await page.getByRole("button", { name: /^Activity/ }).click();
  await expect(page.getByRole("heading", { name: "Activity", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByText("This computer", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("background snapshots preserve an unsaved settings edit", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const name = page.getByRole("textbox", { name: /^Name/ });
  await name.fill("My repaired laptop");
  await page.evaluate(async () => {
    // A visibility change produces the same snapshot refresh as transfer progress.
    const { api } = await import("/src/api.ts");
    await api.setVisibility("trusted");
  });
  await expect(name).toHaveValue("My repaired laptop");
});

test("pairing failure is explained and can be retried", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(async () => {
    const { api } = await import("/src/api.ts");
    api.pairing = async () => { throw new Error("Pairing service unavailable"); };
  });
  await page.getByRole("button", { name: "Your devices", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Pairing service unavailable");
  await expect(page.getByRole("button", { name: "Try again", exact: true })).toBeVisible();
});

test("visibility menu closes with Escape", async ({ page }) => {
  await page.goto("/");
  const button = page.getByRole("button", { name: /Everyone can find this PC/ });
  await button.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(button).toBeFocused();
});

test("sharing and pairing fit a narrow window", async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 720 });
  await page.goto("/");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: "test-results/narrow-share.png", fullPage: true });
  await page.getByRole("button", { name: "Your devices", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Pair a phone", exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test("visibility failures are shown and the choice can be retried", async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 720 });
  await page.goto("/");
  await page.evaluate(async () => {
    const { api } = await import("/src/api.ts");
    const original = api.setVisibility;
    api.setVisibility = async (mode) => {
      api.setVisibility = original;
      throw new Error("Visibility service unavailable");
    };
  });
  const trigger = page.getByRole("button", { name: /Everyone can find this PC/ });
  await trigger.click();
  await page.getByRole("menuitemradio", { name: /Your devices/ }).click();
  await expect(page.getByRole("alert")).toContainText("Visibility service unavailable");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: "test-results/narrow-visibility-error.png", fullPage: true });
  await trigger.click();
  await page.getByRole("menuitemradio", { name: /Your devices/ }).click();
  await expect(page.getByRole("button", { name: /Your devices can find/ })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("visibility menu supports keyboard selection and returns focus", async ({ page }) => {
  await page.goto("/");
  const trigger = page.getByRole("button", { name: /Everyone can find this PC/ });
  await trigger.focus();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitemradio", { name: /^Everyone/ })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitemradio", { name: /^Your devices/ })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
});

test("file actions report errors instead of losing them", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(async () => {
    const { api } = await import("/src/api.ts");
    api.cancel = async () => { throw new Error("Cancel failed; try again"); };
  });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Cancel failed; try again");
});

test("a slow send cannot open a second picker or submit twice", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(async () => {
    const { api } = await import("/src/api.ts");
    await api.stage(["C:\\proof.txt"]);
    (window as any).sendCalls = 0;
    api.send = async () => {
      (window as any).sendCalls++;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    };
  });
  const peer = page.locator("button[data-device]").first();
  await peer.dblclick();
  expect(await page.evaluate(() => (window as any).sendCalls)).toBe(1);
});

test("offer loading failure has a working retry", async ({ page }) => {
  await page.route("**/src/api.ts", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()) + '\napi.offer = async () => { throw new Error("Offer unavailable"); };' });
  });
  await page.goto("/#/offer/demo");
  await expect(page.getByRole("alert")).toContainText("Offer unavailable");
  await page.evaluate(async () => {
    const { api } = await import("/src/api.ts");
    api.offer = async () => null;
  });
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByText("This request has expired.")).toBeVisible();
});

test("download folder save explains when it takes effect", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("textbox", { name: /^Save received files to/ }).fill("D:\\Beam-incoming");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("status")).toContainText(/restart/i);
  await page.getByRole("button", { name: /^Activity/ }).click();
  await expect(page.getByText(/Received files go to/)).toContainText("C:\\Users\\you\\Downloads\\Beam");
});

test("startup does not lose a snapshot while the event listener is registering", async ({ page }) => {
  await page.route("**/src/api.ts", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()) + `
      const listenImmediately = onSnapshot;
      onSnapshot = async (callback) => {
        await new Promise(resolve => setTimeout(resolve, 150));
        return listenImmediately(callback);
      };
      const readSnapshot = api.snapshot;
      api.snapshot = async () => {
        const previous = await readSnapshot();
        await api.setVisibility("trusted");
        return previous;
      };
    ` });
  });
  await page.goto("/");
  await expect(page.getByRole("button", { name: /Your devices can find this PC/ })).toBeVisible();
});

test("an expired offer cannot still be accepted", async ({ page }) => {
  await page.route("**/src/api.ts", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()) + `
      const loadOffer = api.offer;
      api.offer = async (id) => ({ ...await loadOffer(id), expiresMs: Date.now() - 1000 });
    ` });
  });
  await page.goto("/#/offer/demo");
  await expect(page.getByText("This request has expired.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Accept", exact: true })).toHaveCount(0);
});

test("dark mode and reduced motion preserve the sharing controls", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await page.goto("/");
  await page.getByRole("button", { name: /Everyone can find this PC/ }).click();
  await expect(page.getByRole("menuitemradio", { name: /^Everyone/ })).toBeFocused();
  await page.screenshot({ path: "test-results/dark-menu.png", fullPage: true });
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("combobox", { name: /Who can see/ })).toHaveValue("everyone");
  await page.screenshot({ path: "test-results/dark-settings.png", fullPage: true });
});

test("the keep-ready question is asked once and can turn on start with Windows", async ({ page }) => {
  await page.goto("/");
  const prompt = page.getByRole("region", { name: "Keep Beam ready" });
  await expect(prompt).toBeVisible();
  await prompt.getByRole("button", { name: "Turn on" }).click();
  await expect(prompt).toBeHidden();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("switch", { name: /start with windows/i })).toBeChecked();
});

test("not now dismisses the keep-ready question without changing startup", async ({ page }) => {
  await page.goto("/");
  const prompt = page.getByRole("region", { name: "Keep Beam ready" });
  await prompt.getByRole("button", { name: "Not now" }).click();
  await expect(prompt).toBeHidden();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("switch", { name: /start with windows/i })).not.toBeChecked();
});

test("folders trusted devices may save to are saved with the settings", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const group = page.getByRole("group", { name: "Your trusted devices can also save to" });
  await expect(group.getByRole("checkbox", { name: "Desktop" })).toBeChecked();
  await group.getByRole("checkbox", { name: "Videos" }).uncheck();
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Saved" })).toBeVisible();
  const saved = await page.evaluate(async () => (await (await import("/src/api.ts")).api.snapshot()).settings.saveFolders);
  expect(saved).toEqual(["downloads", "desktop", "documents", "pictures"]);
});

test("a pinned folder can be added, saved and removed", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Add a folder…" }).click();
  const folder = String.raw`C:\Users\you\Projects`;
  const pinned = page.getByRole("list", { name: "Pinned folders" });
  await expect(pinned).toContainText(folder);
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Saved" })).toBeVisible();
  const saved = await page.evaluate(async () => (await (await import("/src/api.ts")).api.snapshot()).settings.pinnedFolders);
  expect(saved).toEqual([folder]);
  await page.getByRole("button", { name: `Remove ${folder}` }).click();
  await expect(pinned).toBeHidden();
});

test("the phone app download opens with its link and closes with Escape", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Your devices" }).click();
  await page.getByRole("button", { name: "Get Beam on your phone" }).click();
  const panel = page.getByRole("dialog", { name: "Get Beam on your phone" });
  await expect(panel).toContainText("releases/download/android/BeamShare.apk");
  await expect(panel).toContainText("Coming soon");
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
});
