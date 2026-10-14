import Image from "next/image";
import type { ComponentPropsWithoutRef } from "react";
import type { Shot } from "@/lib/site";
import { placement, stageHeight, stageWidth } from "@/lib/notch";

/**
 * Real captures stacked top-centre on one stage. `visible` is the shot shown
 * before (or without) JavaScript and must not change after mount: GSAP owns
 * opacity and clip-path from then on. `current` only moves the alt text.
 */
export function NotchStage({
  shots,
  visible,
  current = visible,
  preload = false,
  className = "",
  sizes,
  name,
  ...rest
}: Omit<ComponentPropsWithoutRef<"button">, "className"> & {
  shots: Shot[];
  visible: number;
  current?: number;
  preload?: boolean;
  className?: string;
  sizes?: string;
  /** data-stage hook for GSAP. */
  name?: string;
}) {
  const w = stageWidth(shots);
  const h = stageHeight(shots);
  const box = {
    "data-stage": name,
    className: `stage ${className}`,
    style: { width: `min(100%, ${w}px)`, aspectRatio: `${w} / ${h}` },
  };
  const images = shots.map((shot, i) => (
    <Image
      key={shot.src}
      src={shot.src}
      alt={i === current ? shot.alt : ""}
      aria-hidden={i === current ? undefined : true}
      width={Math.round(shot.w)}
      height={Math.round(shot.h)}
      preload={preload}
      sizes={sizes}
      data-shot={i}
      draggable={false}
      style={{ ...placement(shot, w), opacity: i === visible ? 1 : 0 }}
    />
  ));
  // A button when it can be clicked (onClick passed), otherwise a plain box.
  return rest.onClick ? (
    <button type="button" {...rest} {...box}>
      {images}
    </button>
  ) : (
    <div {...box}>{images}</div>
  );
}
