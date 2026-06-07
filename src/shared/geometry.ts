// Camera / coordinate primitives shared across the shell and renderer adapter.
//
// These are render-time camera/coords, not domain model: they describe where the
// viewport sits and how a screen point maps to world space. They are deliberately
// pointer-width-agnostic (plain JS numbers, no 32-bit address assumptions) so a
// future 64-bit wasm port is a target-triple flip.

/** A point in world (canvas) space. */
export type WorldPoint = {
  x: number;
  y: number;
};

/** An axis-aligned rectangle in world (canvas) space. */
export type WorldRect = WorldPoint & {
  width: number;
  height: number;
};

/** The viewport camera: world-space pan offset plus zoom. */
export type CameraState = {
  x: number;
  y: number;
  zoom: number;
};
