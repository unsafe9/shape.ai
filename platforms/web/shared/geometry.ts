// Render-time camera/coordinate primitives (not domain model). Pointer-width-agnostic (plain JS
// numbers, no 32-bit address assumptions) so a future 64-bit wasm port is a target-triple flip.

// A point in world (canvas) space.
export type WorldPoint = {
  x: number;
  y: number;
};

// An axis-aligned rectangle in world (canvas) space.
export type WorldRect = WorldPoint & {
  width: number;
  height: number;
};

// The viewport camera: world-space pan offset plus zoom.
export type CameraState = {
  x: number;
  y: number;
  zoom: number;
};
