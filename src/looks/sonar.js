// The wave the core sends out every theme.sonar.every seconds (theme.sonar).
// Each style:
//   start(theme, { center, floorY, color })  one wave from the core (center
//                                            is null without one): the
//                                            object to add to the scene
//   step(wave, dt)                           moves it on; false once it's
//                                            gone
//   dispose(wave)                            frees what start made
import * as THREE from "three";
import { Additive, fresnelMaterial } from "../fx.js";

const floorRing = (color, floorY, width = 0.015) => {
  const ring = new THREE.Mesh(
    new THREE.RingGeometry(1 - width, 1, 192),
    new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0, side: THREE.DoubleSide, depthWrite: false, blending: Additive }),
  );
  ring.rotation.x = -Math.PI / 2;
  ring.position.y = floorY + 1;
  return ring;
};

const disposeAll = (object) =>
  object.traverse((o) => {
    o.geometry?.dispose();
    o.material?.dispose();
  });

const easeOut = (k, p = 2.4) => 1 - (1 - k) ** p;
const clamp01 = (x) => Math.min(1, Math.max(0, x));

export const SONAR = {
  // A glowing shell that swells from the core, and a ring across the floor.
  ring: {
    start(theme, { center, floorY, color }) {
      const shell = new THREE.Mesh(new THREE.SphereGeometry(1, 64, 48), fresnelMaterial(color, 0.9, 1.7));
      const ring = new THREE.Mesh(
        new THREE.RingGeometry(0.985, 1, 192),
        new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.6, side: THREE.DoubleSide, depthWrite: false, blending: Additive }),
      );
      ring.rotation.x = -Math.PI / 2;
      ring.position.y = floorY + 1;
      if (center) shell.position.copy(center);
      const object = new THREE.Group();
      object.add(shell, ring);
      return { object, shell, ring, t: 0 };
    },
    step(wave, dt) {
      wave.t += dt / 5.5;
      if (wave.t >= 1) return false;
      const e = 1 - (1 - wave.t) ** 2.4;
      wave.shell.scale.setScalar(12 + e * 520);
      wave.shell.material.uniforms.opacity.value = 0.7 * (1 - wave.t) ** 1.5;
      wave.ring.scale.setScalar(12 + e * 440);
      wave.ring.material.opacity = 0.55 * (1 - wave.t) ** 1.2;
      return true;
    },
    dispose(wave) {
      for (const m of [wave.shell, wave.ring]) {
        m.geometry.dispose();
        m.material.dispose();
      }
    },
  },

  // Rings spreading across the floor one after another, like a stone
  // dropped in a still pool.
  ripple: {
    start(theme, { floorY, color }) {
      const rings = [0, 1, 2, 3].map(() => floorRing(color, floorY, 0.01));
      const object = new THREE.Group();
      object.add(...rings);
      return { object, rings, t: 0 };
    },
    step(wave, dt) {
      wave.t += dt / 6.5;
      if (wave.t >= 1) return false;
      wave.rings.forEach((ring, i) => {
        const k = clamp01((wave.t - i * 0.09) / 0.7);
        ring.visible = k > 0;
        ring.scale.setScalar(6 + easeOut(k, 1.8) * 440 * (1 - i * 0.06));
        ring.material.opacity = 0.5 * (1 - k) ** 1.4 * (1 - i * 0.18);
      });
      return true;
    },
    dispose: (wave) => disposeAll(wave.object),
  },

  // Sonar as a ship has it: a sharp ping across the floor and a flash at
  // the core, then a fainter echo coming back in.
  ping: {
    start(theme, { center, floorY, color }) {
      const ping = floorRing(color, floorY, 0.006);
      const echo = floorRing(color, floorY, 0.012);
      const flash = new THREE.Mesh(new THREE.SphereGeometry(1, 48, 32), fresnelMaterial(color, 0, 1.4));
      if (center) flash.position.copy(center);
      const object = new THREE.Group();
      object.add(ping, echo, flash);
      return { object, ping, echo, flash, t: 0 };
    },
    step(wave, dt) {
      wave.t += dt;
      const out = 2.4, back = 3.2;
      if (wave.t >= out + back) return false;
      const k = clamp01(wave.t / out);
      wave.ping.visible = wave.t < out;
      wave.ping.scale.setScalar(4 + easeOut(k, 3) * 440);
      wave.ping.material.opacity = 0.9 * (1 - k) ** 0.7;
      const f = clamp01(wave.t / 0.9);
      wave.flash.visible = f < 1;
      wave.flash.scale.setScalar(10 + f * 70);
      wave.flash.material.uniforms.opacity.value = 0.8 * (1 - f) ** 2;
      const b = clamp01((wave.t - out) / back);
      wave.echo.visible = b > 0;
      wave.echo.scale.setScalar(440 - (1 - (1 - b) ** 1.5) * 430);
      wave.echo.material.opacity = 0.32 * Math.sin(Math.PI * b);
      return true;
    },
    dispose: (wave) => disposeAll(wave.object),
  },
};
