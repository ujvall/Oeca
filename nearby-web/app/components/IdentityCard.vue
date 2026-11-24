<!-- oeca: this browser's identity as a brushed-metal card: Oeca Nearby, the
     notch, the name the PC sees and the end of this browser's key
     fingerprint (the same short form a PC lists for a trusted browser). A
     picture of facts already on the page; it proves nothing by itself. -->
<template>
  <div ref="host" class="tilt w-full max-w-[21rem]" @pointermove="track" @pointerleave="rest" @pointercancel="rest">
  <div class="metal relative flex aspect-[1.586] w-full flex-col justify-between overflow-hidden rounded-[24px] p-5 text-start">
    <div class="flex items-start justify-between">
      <span class="metal-ink display text-[19px] font-semibold leading-none tracking-[-0.02em]">
        oeca<span class="font-normal text-black/45"> hop</span>
      </span>
      <!-- The notch, inlaid -->
      <span class="flex h-6 w-[60px] items-center justify-end rounded-full bg-black pe-2 shadow-[inset_0_1px_2px_rgb(0_0_0/0.6),0_1px_0_rgb(255_255_255/0.5)]" aria-hidden="true">
        <span class="size-1.5 rounded-full bg-signal" />
      </span>
    </div>
    <div class="flex items-end justify-between gap-3">
      <span class="min-w-0">
        <span class="metal-ink sub block text-[12px] !text-black/50">{{ props.caption }}</span>
        <span class="metal-ink display mt-1 flex items-center gap-1.5 text-[20px] font-medium min-[360px]:text-[24px] leading-tight tracking-[-0.02em]">
          <span class="truncate">{{ props.name }}</span>
          <Glyph v-if="props.editable" name="pencil" class="size-4 shrink-0 text-black/40" />
        </span>
      </span>
      <span v-if="props.digits" class="metal-ink shrink-0 pb-1 text-[13px] font-medium tabular-nums tracking-[0.06em] min-[360px]:text-[14px] text-black/60" :aria-label="`Key ending ${props.digits}`">
        •••• {{ props.digits }}
      </span>
    </div>
  </div>
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{ name: string; caption: string; digits?: string; editable?: boolean }>();

// The card leans toward the pointer (a few degrees) and its highlight follows.
// Custom properties through the CSSOM: the policy refuses style attributes
const host = ref<HTMLElement | null>(null);
const still = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

function track(e: PointerEvent) {
  const el = host.value;
  if (!el || still) return;
  const r = el.getBoundingClientRect();
  const x = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  const y = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
  el.classList.add("is-tracking");
  el.style.setProperty("--ry", `${((x - 0.5) * 14).toFixed(2)}deg`);
  el.style.setProperty("--rx", `${((0.5 - y) * 10).toFixed(2)}deg`);
  el.style.setProperty("--mx", `${(x * 100).toFixed(1)}%`);
  el.style.setProperty("--my", `${(y * 100).toFixed(1)}%`);
  el.style.setProperty("--glare", "0.9");
}

function rest() {
  const el = host.value;
  if (!el) return;
  el.classList.remove("is-tracking");
  for (const p of ["--rx", "--ry", "--mx", "--my", "--glare"]) el.style.removeProperty(p);
}
</script>
