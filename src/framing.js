// Where the camera stands to frame a set of points whole, looking one way,
// in the part of the screen the panels and the alert strip leave free (fit
// in scene.js). Plain math on plain numbers, so it can be tested without a
// page.
//
// The points come in the camera's frame for that way of looking, about any
// middle: x to the right, y up, z toward the camera, and r how far each one
// reaches (its glow). The free part of the screen comes as the tangents of
// the angles from the screen's middle to its four edges, as a field of
// view would: a whole screen's top and bottom edges are at tan(fov / 2).
// An edge across the middle has a negative tangent.

// The tangents of a free rectangle's edges, for a camera with vertical
// field of view `fov` (degrees) on a `width` × `height` screen less
// `insets` in pixels from each edge and a `margin` inside them. Too little
// room left across or up and down, and that way frames on the whole screen.
export function freeTangents({ width, height, fov, insets = {}, margin = 24, least = 240 }) {
  const ins = { left: 0, right: 0, top: 0, bottom: 0, ...insets };
  const perPx = Math.tan((fov * Math.PI) / 360) / (height / 2);
  const room = (whole, a, b) => (whole - a - b - 2 * margin >= least ? [(whole - a - b) / 2 - margin, (a - b) / 2] : [whole / 2 - margin, 0]);
  // Half the free width and height, and how far the free middle sits right
  // of the screen's middle (x) and below it (y).
  const [halfW, x] = room(width, ins.left, ins.right);
  const [halfH, y] = room(height, ins.top, ins.bottom);
  return { left: (halfW - x) * perPx, right: (halfW + x) * perPx, top: (halfH - y) * perPx, bottom: (halfH + y) * perPx };
}

// How far back the camera stands (d) and how far right and up of the
// points' middle it looks (a, b): the nearest place that sees every point,
// glow and all, inside the free part of the screen, at least `min` back.
// For each side, a point's reach that way plus its nearness times that
// side's tangent bounds where the camera can look; across both sides of a
// pair, that bounds how near it can stand. Null with no points.
export function frameFree(points, { left, right, top, bottom }, { min = 0 } = {}) {
  if (!points.length) return null;
  let toRight = -Infinity, toLeft = -Infinity, toTop = -Infinity, toBottom = -Infinity, nearest = -Infinity;
  for (const p of points) {
    const r = p.r ?? 0;
    toRight = Math.max(toRight, p.x + r + right * p.z);
    toLeft = Math.max(toLeft, -p.x + r + left * p.z);
    toTop = Math.max(toTop, p.y + r + top * p.z);
    toBottom = Math.max(toBottom, -p.y + r + bottom * p.z);
    nearest = Math.max(nearest, p.z + r);
  }
  const d = Math.max((toRight + toLeft) / (left + right), (toTop + toBottom) / (top + bottom), nearest + 1, min);
  // In the middle of where each pair of sides lets it look.
  const a = (toRight - right * d - toLeft + left * d) / 2;
  const b = (toTop - top * d - toBottom + bottom * d) / 2;
  return { d, a, b };
}
