// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.mark-scene
// Responsibility:   The three-dimensional mark of the home page drawn with WebGL on a transparent ground: the two chevrons extruded from the path data of the identity, in brushed metal under a studio environment. The colours are the mark colours of the current theme and follow a change of theme. The chevrons part and close again in a slow cycle (a change and its undo) and turn towards the pointer. The scene renders only while it is in view and the page is visible.
// Dependencies:     three (MIT), website.src.components.brand.brand-data

import {
  ACESFilmicToneMapping,
  CanvasTexture,
  Color,
  DirectionalLight,
  ExtrudeGeometry,
  HemisphereLight,
  Group,
  Mesh,
  MeshPhysicalMaterial,
  PerspectiveCamera,
  PMREMGenerator,
  SRGBColorSpace,
  Scene,
  WebGLRenderer,
  type Shape,
} from 'three';
import { SVGLoader } from 'three/examples/jsm/loaders/SVGLoader.js';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { markPaths, markViewBox } from '../../components/brand/brand-data';

const cycleSeconds = 9;
const cameraDistance = 285;

function shapesOf(path: string): Shape[] {
  const [x, y, width, height] = markViewBox.split(' ').map(Number);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${x} ${y} ${width} ${height}"><path d="${path}"/></svg>`;
  const data = new SVGLoader().parse(svg);
  return data.paths.flatMap((shapePath) => SVGLoader.createShapes(shapePath));
}

function chevron(path: string, material: MeshPhysicalMaterial): Mesh {
  const geometry = new ExtrudeGeometry(shapesOf(path), {
    depth: 7,
    bevelEnabled: true,
    bevelThickness: 1.1,
    bevelSize: 0.9,
    bevelSegments: 6,
    curveSegments: 18,
  });
  const [x, y, width, height] = markViewBox.split(' ').map(Number);
  geometry.translate(-(x + width / 2), -(y + height / 2), -3.5);
  return new Mesh(geometry, material);
}

function brushedTexture(): CanvasTexture {
  const canvas = document.createElement('canvas');
  canvas.width = 512;
  canvas.height = 512;
  const context = canvas.getContext('2d');
  if (context) {
    context.fillStyle = '#808080';
    context.fillRect(0, 0, 512, 512);
    for (let line = 0; line < 2400; line += 1) {
      const shade = 110 + Math.floor(Math.random() * 40);
      context.strokeStyle = `rgba(${shade},${shade},${shade},0.35)`;
      context.lineWidth = Math.random() * 1.2;
      const y = Math.random() * 512;
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(512, y + (Math.random() - 0.5) * 4);
      context.stroke();
    }
  }
  const texture = new CanvasTexture(canvas);
  texture.anisotropy = 4;
  return texture;
}

function easeInOut(value: number): number {
  return value < 0.5 ? 4 * value * value * value : 1 - Math.pow(-2 * value + 2, 3) / 2;
}

export function startMarkScene(): void {
  const stages = Array.from(document.querySelectorAll<HTMLElement>('[data-mark-scene]'));
  for (const stage of stages) {
    const canvas = stage.querySelector<HTMLCanvasElement>('canvas');
    if (!canvas) {
      continue;
    }
    let renderer: WebGLRenderer;
    try {
      renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: 'low-power' });
    } catch {
      continue;
    }
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.toneMapping = ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.05;

    const scene = new Scene();
    const environment = new PMREMGenerator(renderer);
    scene.environment = environment.fromScene(new RoomEnvironment(), 0.04).texture;

    const camera = new PerspectiveCamera(32, 1, 1, 1000);
    camera.position.set(0, 0, cameraDistance);

    const fill = new HemisphereLight(0xf6f7f8, 0x0b0c0e, 1.1);
    scene.add(fill);
    const key = new DirectionalLight(0xfff1d6, 2.6);
    key.position.set(60, 80, 120);
    scene.add(key);
    const rim = new DirectionalLight(0x9fd8ff, 0.8);
    rim.position.set(-90, -40, -60);
    scene.add(rim);

    const brushed = brushedTexture();
    const backMaterial = new MeshPhysicalMaterial({ metalness: 0.92, roughness: 0.38, roughnessMap: brushed, clearcoat: 0.35, clearcoatRoughness: 0.25, envMapIntensity: 1.9 });
    const forwardMaterial = new MeshPhysicalMaterial({ metalness: 0.78, roughness: 0.3, clearcoat: 0.6, clearcoatRoughness: 0.12, envMapIntensity: 1.8 });

    // The path data has the y axis of SVG pointing down; the inner group mirrors it, and three.js keeps the faces correct for a mirrored object.
    const mark = new Group();
    const drawing = new Group();
    drawing.scale.set(1, -1, 1);
    const back = chevron(markPaths.silver, backMaterial);
    const forward = chevron(markPaths.gold, forwardMaterial);
    drawing.add(back, forward);
    mark.add(drawing);
    scene.add(mark);

    // The back chevron takes the mark colour of the theme in force and the forward chevron the gold of the identity ; a dark back chevron means the light theme.
    const applyTheme = (): void => {
      const style = getComputedStyle(stage);
      const backColour = new Color(style.getPropertyValue('--mark-back').trim() || '#dde1e6');
      const forwardColour = new Color(style.getPropertyValue('--mark-scene-forward').trim() || '#c9a15a');
      const lightGround = backColour.getHSL({ h: 0, s: 0, l: 0 }).l < 0.5;
      backMaterial.color.copy(backColour);
      backMaterial.envMapIntensity = lightGround ? 2.6 : 1.9;
      forwardMaterial.color.copy(forwardColour);
    };
    applyTheme();
    new MutationObserver(applyTheme).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);

    // The mark turns towards the pointer anywhere over the section that holds the stage.
    const pointer = { x: 0, y: 0, targetX: 0, targetY: 0 };
    const surface = stage.closest('section') ?? stage;
    surface.addEventListener('pointermove', (event) => {
      const box = stage.getBoundingClientRect();
      const x = ((event as PointerEvent).clientX - (box.left + box.width / 2)) / box.width;
      const y = ((event as PointerEvent).clientY - (box.top + box.height / 2)) / box.height;
      pointer.targetX = Math.max(-1, Math.min(1, x));
      pointer.targetY = Math.max(-1, Math.min(1, y));
    });
    surface.addEventListener('pointerleave', () => {
      pointer.targetX = 0;
      pointer.targetY = 0;
    });

    const resize = (): void => {
      const { width, height } = canvas.getBoundingClientRect();
      if (width === 0 || height === 0) {
        return;
      }
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    };
    resize();
    new ResizeObserver(resize).observe(canvas);

    let running = false;
    let visible = false;
    let last = performance.now();
    let elapsed = 0;

    const frame = (now: number): void => {
      if (!running) {
        return;
      }
      const delta = Math.min((now - last) / 1000, 0.05);
      last = now;
      elapsed += delta;

      pointer.x += (pointer.targetX - pointer.x) * 0.06;
      pointer.y += (pointer.targetY - pointer.y) * 0.06;

      const phase = (elapsed % cycleSeconds) / cycleSeconds;
      const open = phase < 0.5 ? easeInOut(phase * 2) : easeInOut((1 - phase) * 2);
      back.position.set(-open * 7, -open * 3, -open * 6);
      forward.position.set(open * 7, open * 3, open * 6);

      mark.rotation.y = -0.42 + Math.sin(elapsed * 0.35) * 0.18 + pointer.x * 0.35;
      mark.rotation.x = 0.08 + Math.sin(elapsed * 0.27) * 0.06 + pointer.y * 0.22;
      mark.position.y = Math.sin(elapsed * 0.8) * 1.6;

      renderer.render(scene, camera);
      requestAnimationFrame(frame);
    };

    const update = (): void => {
      const shouldRun = visible && document.visibilityState === 'visible';
      if (shouldRun && !running) {
        running = true;
        last = performance.now();
        requestAnimationFrame(frame);
      } else if (!shouldRun) {
        running = false;
      }
    };

    new IntersectionObserver((entries) => {
      visible = entries.some((entry) => entry.isIntersecting);
      update();
    }).observe(stage);
    document.addEventListener('visibilitychange', update);

    renderer.render(scene, camera);
    stage.classList.add('is-live');
  }
}
