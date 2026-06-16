// Meta-guard: proves the P4 "no TS product UI" guards in thin-shell-endstate.test.ts actually FIRE when
// TS product UI is reintroduced — not merely pass on the current clean tree. Each P4 assertion is mirrored
// here as the same pure predicate the endstate test runs, then checked twice: it must PASS on the real
// shell source (keeping this meta-test in lockstep with the live guard) AND FAIL on a synthetic
// reintroduction (a resurrected component, a re-added .svelte import, a status-only relay, a rebuilt panel
// markup, a key-literal UI branch, or a resurrected product-UI selector). If a guard is ever weakened so a
// reintroduction slips through, the corresponding "fires" case below fails the build.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const exists = (rel: string) => existsSync(fileURLToPath(new URL(rel, import.meta.url)));

const appSource = read("../ui/App.svelte");
const stylesSource = read("../styles.css");
const canvasHostSource = read("../controller/canvasHost.ts");

// ----- the P4 guard predicates, verbatim with the endstate test ----------------------------------------

// (1) A product-UI component file must not exist.
const componentExists = (path: string): boolean => exists(path);

// (2) The only `*.svelte` import the shell keeps is the surface provider.
const productSvelteImports = (src: string): string[] =>
  src.split("\n").filter((line) => /import\s+.*from\s+"\.\/[^"]+\.svelte"/.test(line));

// (3) The shell routes the UI through the model+intent seam (feed + typed intent switch), not a status relay.
const routesThroughSeam = (app: string, host: string): boolean =>
  app.includes("host?.setUiModel(") &&
  app.includes("function handleUiIntent(intent: UiIntent)") &&
  app.includes('case "command":') &&
  app.includes('case "inspectorEdit":') &&
  !host.includes("`ui-action ${event.action.type}");

// (4) The shell markup (after `</script>`) builds no product-UI panel.
const PRODUCT_MARKUP_LITERALS = [
  'class="toolbar',
  'class="inspector',
  'class="settings-modal',
  'class="node-context-menu',
  'class="template-library',
  'class="diagnostics-panel'
];
const markupHasProductPanel = (app: string): boolean => {
  const markup = app.slice(app.indexOf("</script>"));
  return PRODUCT_MARKUP_LITERALS.some((literal) => markup.includes(literal));
};

// (5) The uiKey forward must exist and must not branch on a key literal.
const uiKeyForwardBranchesOnLiteral = (app: string): boolean => {
  const forwardLine = app
    .split("\n")
    .find((line) => line.includes("host?.uiKey({ key: event.key, text: keyChar(event),"));
  // A removed forward OR a key-literal branch is a violation.
  return forwardLine === undefined || /event\.key\s*===/.test(forwardLine);
};

// (6) styles.css holds none of the deleted product-UI selectors.
const PRODUCT_SELECTORS = [
  ".toolbar-remote",
  ".inspector-panel",
  ".settings-modal",
  ".node-context-menu",
  ".template-library",
  ".diagnostics-panel"
];
const stylesHaveProductSelector = (css: string): boolean =>
  PRODUCT_SELECTORS.some((selector) => css.includes(selector));

describe("P4 guards stay in lockstep with the clean shell (the predicates pass as written)", () => {
  it("no product-UI component file exists", () => {
    for (const path of [
      "../ui/Toolbar.svelte",
      "../ui/SettingsModal.svelte",
      "../ui/InspectorPanel.svelte",
      "../ui/ContextMenu.svelte",
      "../ui/TemplatePopup.svelte",
      "../ui/PeerCursors.svelte"
    ]) {
      expect(componentExists(path)).toBe(false);
    }
  });

  it("the shell imports exactly one .svelte component (the surface provider)", () => {
    const imports = productSvelteImports(appSource);
    expect(imports.length).toBe(1);
    expect(imports[0]).toContain("./ShapeCanvasHost.svelte");
  });

  it("the shell routes through the model+intent seam", () => {
    expect(routesThroughSeam(appSource, canvasHostSource)).toBe(true);
  });

  it("the shell markup builds no product-UI panel", () => {
    expect(markupHasProductPanel(appSource)).toBe(false);
  });

  it("the uiKey forward exists and does not branch on a key literal", () => {
    expect(uiKeyForwardBranchesOnLiteral(appSource)).toBe(false);
  });

  it("styles.css holds no product-UI selector", () => {
    expect(stylesHaveProductSelector(stylesSource)).toBe(false);
  });
});

describe("P4 guards FIRE on a TS product-UI reintroduction (the falsifiable proof)", () => {
  it("a resurrected component file would be caught (existsSync flips true)", () => {
    // The guard reads existsSync per path. A resurrected file makes existsSync return true, which the
    // endstate `.toBe(false)` assertion rejects. Prove the predicate distinguishes present from absent:
    // a path that DOES exist (this very test file) must read as present.
    expect(componentExists("./p4-guard-fires.test.ts")).toBe(true);
    expect(componentExists("../ui/Toolbar.svelte")).toBe(false);
  });

  it("a re-added product-UI .svelte import is caught (import count exceeds one)", () => {
    const reintroduced = `${appSource}\n  import Toolbar from "./Toolbar.svelte";`;
    const imports = productSvelteImports(reintroduced);
    expect(imports.length).toBeGreaterThan(1);
    // And the surface-provider sole-import invariant the endstate test asserts is now false.
    expect(imports.length === 1).toBe(false);
  });

  it("dropping the model+intent seam for a status-only relay is caught", () => {
    // Strip the feed: the seam predicate goes false.
    const noFeed = appSource.replace("host?.setUiModel(", "host?.__removed_setUiModel(");
    expect(routesThroughSeam(noFeed, canvasHostSource)).toBe(false);
    // Reintroducing the status-only ui-action relay in the host is also caught.
    const statusRelay = "status = `ui-action ${event.action.type}`;";
    expect(routesThroughSeam(appSource, `${canvasHostSource}\n${statusRelay}`)).toBe(false);
  });

  it("a rebuilt product-UI panel in the markup is caught (each panel literal)", () => {
    const scriptEnd = appSource.indexOf("</script>");
    for (const literal of PRODUCT_MARKUP_LITERALS) {
      const reintroduced =
        appSource.slice(0, scriptEnd) +
        appSource.slice(scriptEnd).replace("<div class=\"app-shell\">", `<div ${literal}-remote">`);
      expect(markupHasProductPanel(reintroduced), `markup guard must catch ${literal}`).toBe(true);
    }
  });

  it("a key-literal UI behavior branch on the uiKey forward is caught", () => {
    // Splice a key-literal comparison onto the forward line — the exact leak the catalog single-source rule
    // forbids. The guard must trip.
    const branched = appSource.replace(
      "host?.uiKey({ key: event.key, text: keyChar(event),",
      "event.key === \"t\" ? null : host?.uiKey({ key: event.key, text: keyChar(event),"
    );
    expect(uiKeyForwardBranchesOnLiteral(branched)).toBe(true);
    // Removing the forward entirely is also a violation.
    const removed = appSource.replace("host?.uiKey({ key: event.key, text: keyChar(event),", "// gone");
    expect(uiKeyForwardBranchesOnLiteral(removed)).toBe(true);
  });

  it("a resurrected product-UI selector in styles.css is caught (each selector)", () => {
    for (const selector of PRODUCT_SELECTORS) {
      const reintroduced = `${stylesSource}\n${selector} { color: red; }`;
      expect(stylesHaveProductSelector(reintroduced), `style guard must catch ${selector}`).toBe(true);
    }
  });
});
