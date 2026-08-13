import { PhysicalPosition, getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Drag the undecorated HUD anywhere on screen.
 * Prefer OS startDragging; fall back to pointer-delta positioning
 * because transparent accessory windows often ignore CSS drag regions.
 */
export function bindWindowDrag(el: HTMLElement): () => void {
  const win = getCurrentWindow();
  let dragging = false;
  let originX = 0;
  let originY = 0;
  let startX = 0;
  let startY = 0;
  let usedOsDrag = false;

  const onDown = async (e: PointerEvent) => {
    if (e.button !== 0) return;
    usedOsDrag = false;
    try {
      await win.startDragging();
      usedOsDrag = true;
      return;
    } catch {
      // fall through to manual drag
    }
    try {
      const pos = await win.outerPosition();
      dragging = true;
      originX = e.screenX;
      originY = e.screenY;
      startX = pos.x;
      startY = pos.y;
      el.setPointerCapture(e.pointerId);
    } catch {
      dragging = false;
    }
  };

  const onMove = async (e: PointerEvent) => {
    if (!dragging || usedOsDrag) return;
    const x = Math.round(startX + (e.screenX - originX));
    const y = Math.round(startY + (e.screenY - originY));
    try {
      await win.setPosition(new PhysicalPosition(x, y));
    } catch {
      // ignore mid-drag errors
    }
  };

  const onUp = (e: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    try {
      el.releasePointerCapture(e.pointerId);
    } catch {
      // already released
    }
  };

  el.addEventListener("pointerdown", onDown);
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerup", onUp);
  el.addEventListener("pointercancel", onUp);

  return () => {
    el.removeEventListener("pointerdown", onDown);
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerup", onUp);
    el.removeEventListener("pointercancel", onUp);
  };
}
