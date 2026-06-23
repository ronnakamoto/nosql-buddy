import { test, expect } from "@playwright/test";

// NoSQLBuddy smoke test: the frontend shell renders.
// In a browser (no Tauri runtime), the IPC calls fail and are caught by App's
// try/catch, so the shell still renders. Full IPC E2E requires tauri-driver.
test("app shell renders", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".app__titlebar-brand")).toHaveText("NoSQLBuddy");
  await expect(page.getByRole("button", { name: /New connection/i }).first()).toBeVisible();
});
