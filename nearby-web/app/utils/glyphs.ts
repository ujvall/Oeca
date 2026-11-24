// oeca: the interface icons: Phosphor Icons 2.1 (MIT, @phosphor-icons/core,
// a dev dependency). Only these files are bundled, as static SVG; Bold for
// controls, Fill for badges, stickers and feature icons.
import alertB from "@phosphor-icons/core/assets/bold/warning-bold.svg?raw";
import arcB from "@phosphor-icons/core/assets/bold/circle-notch-bold.svg?raw";
import arrowB from "@phosphor-icons/core/assets/bold/arrow-up-right-bold.svg?raw";
import backB from "@phosphor-icons/core/assets/bold/caret-left-bold.svg?raw";
import boltB from "@phosphor-icons/core/assets/bold/lightning-bold.svg?raw";
import checkB from "@phosphor-icons/core/assets/bold/check-bold.svg?raw";
import chevronB from "@phosphor-icons/core/assets/bold/caret-right-bold.svg?raw";
import closeB from "@phosphor-icons/core/assets/bold/x-bold.svg?raw";
import contrastB from "@phosphor-icons/core/assets/bold/circle-half-bold.svg?raw";
import cubeB from "@phosphor-icons/core/assets/bold/cube-bold.svg?raw";
import desktopB from "@phosphor-icons/core/assets/bold/desktop-bold.svg?raw";
import devicesB from "@phosphor-icons/core/assets/bold/devices-bold.svg?raw";
import downloadB from "@phosphor-icons/core/assets/bold/download-simple-bold.svg?raw";
import eyeB from "@phosphor-icons/core/assets/bold/eye-bold.svg?raw";
import fileB from "@phosphor-icons/core/assets/bold/file-bold.svg?raw";
import filmB from "@phosphor-icons/core/assets/bold/film-strip-bold.svg?raw";
import globeB from "@phosphor-icons/core/assets/bold/globe-simple-bold.svg?raw";
import imageB from "@phosphor-icons/core/assets/bold/image-bold.svg?raw";
import infoB from "@phosphor-icons/core/assets/bold/info-bold.svg?raw";
import keyB from "@phosphor-icons/core/assets/bold/key-bold.svg?raw";
import lockB from "@phosphor-icons/core/assets/bold/lock-simple-bold.svg?raw";
import moonB from "@phosphor-icons/core/assets/bold/moon-bold.svg?raw";
import nearbyB from "@phosphor-icons/core/assets/bold/wifi-high-bold.svg?raw";
import pencilB from "@phosphor-icons/core/assets/bold/pencil-simple-bold.svg?raw";
import personB from "@phosphor-icons/core/assets/bold/user-bold.svg?raw";
import phoneB from "@phosphor-icons/core/assets/bold/device-mobile-bold.svg?raw";
import plusB from "@phosphor-icons/core/assets/bold/plus-bold.svg?raw";
import powerB from "@phosphor-icons/core/assets/bold/power-bold.svg?raw";
import refreshB from "@phosphor-icons/core/assets/bold/arrow-clockwise-bold.svg?raw";
import sendB from "@phosphor-icons/core/assets/bold/arrow-up-bold.svg?raw";
import settingsB from "@phosphor-icons/core/assets/bold/sliders-horizontal-bold.svg?raw";
import shieldB from "@phosphor-icons/core/assets/bold/shield-check-bold.svg?raw";
import sunB from "@phosphor-icons/core/assets/bold/sun-bold.svg?raw";
import trashB from "@phosphor-icons/core/assets/bold/trash-bold.svg?raw";
import alertF from "@phosphor-icons/core/assets/fill/warning-fill.svg?raw";
import cubeF from "@phosphor-icons/core/assets/fill/cube-fill.svg?raw";
import desktopF from "@phosphor-icons/core/assets/fill/desktop-fill.svg?raw";
import devicesF from "@phosphor-icons/core/assets/fill/devices-fill.svg?raw";
import eyeF from "@phosphor-icons/core/assets/fill/eye-fill.svg?raw";
import fileF from "@phosphor-icons/core/assets/fill/file-fill.svg?raw";
import filmF from "@phosphor-icons/core/assets/fill/film-strip-fill.svg?raw";
import globeF from "@phosphor-icons/core/assets/fill/globe-simple-fill.svg?raw";
import imageF from "@phosphor-icons/core/assets/fill/image-fill.svg?raw";
import infoF from "@phosphor-icons/core/assets/fill/info-fill.svg?raw";
import keyF from "@phosphor-icons/core/assets/fill/key-fill.svg?raw";
import lockF from "@phosphor-icons/core/assets/fill/lock-simple-fill.svg?raw";
import nearbyF from "@phosphor-icons/core/assets/fill/wifi-high-fill.svg?raw";
import phoneF from "@phosphor-icons/core/assets/fill/device-mobile-fill.svg?raw";
import powerF from "@phosphor-icons/core/assets/fill/power-fill.svg?raw";
import shieldF from "@phosphor-icons/core/assets/fill/shield-check-fill.svg?raw";

/** The SVG's contents, without its <svg> wrapper (Glyph draws that). */
const inner = (svg: string) => svg.replace(/^\s*<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");

const BOLD_RAW = {
  alert: alertB,
  arc: arcB,
  arrow: arrowB,
  back: backB,
  bolt: boltB,
  check: checkB,
  chevron: chevronB,
  close: closeB,
  contrast: contrastB,
  cube: cubeB,
  desktop: desktopB,
  devices: devicesB,
  download: downloadB,
  eye: eyeB,
  file: fileB,
  film: filmB,
  globe: globeB,
  image: imageB,
  info: infoB,
  key: keyB,
  lock: lockB,
  moon: moonB,
  nearby: nearbyB,
  pencil: pencilB,
  person: personB,
  phone: phoneB,
  plus: plusB,
  power: powerB,
  refresh: refreshB,
  send: sendB,
  settings: settingsB,
  shield: shieldB,
  sun: sunB,
  trash: trashB,
};
export type GlyphName = keyof typeof BOLD_RAW;

const FILL_RAW: Partial<Record<GlyphName, string>> = {
  alert: alertF,
  cube: cubeF,
  desktop: desktopF,
  devices: devicesF,
  eye: eyeF,
  file: fileF,
  film: filmF,
  globe: globeF,
  image: imageF,
  info: infoF,
  key: keyF,
  lock: lockF,
  nearby: nearbyF,
  phone: phoneF,
  power: powerF,
  shield: shieldF,
};

export const BOLD = Object.fromEntries(Object.entries(BOLD_RAW).map(([k, v]) => [k, inner(v)])) as Record<GlyphName, string>;
export const FILL = Object.fromEntries(Object.entries(FILL_RAW).map(([k, v]) => [k, inner(v)])) as Partial<Record<GlyphName, string>>;
