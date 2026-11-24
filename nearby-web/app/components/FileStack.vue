<!-- oeca: files as a small fanned stack of cards, each marked with its
     extension (a dangerous type in amber), with the count. Decorative: the
     list beside it carries the names. -->
<template>
  <div class="relative mx-auto flex h-[148px] w-full max-w-[240px] items-end justify-center" aria-hidden="true">
    <TransitionGroup name="arrive">
      <div
        v-for="(f, i) in shown"
        :key="f.key"
        class="object absolute bottom-2 flex h-[128px] w-[96px] flex-col justify-between rounded-[18px] p-3 transition-transform duration-500 [transition-timing-function:cubic-bezier(0.32,0.72,0,1)]"
        :class="POSE[shown.length - 1]?.[i]"
      >
        <span class="h-1.5 w-10 rounded-full bg-line/10" />
        <span class="space-y-1.5">
          <span class="block h-1 w-14 rounded-full bg-line/[0.07]" />
          <span class="block h-1 w-10 rounded-full bg-line/[0.07]" />
        </span>
        <span
          class="num self-start truncate rounded-[8px] px-2 py-1 text-[12px] font-semibold uppercase leading-none tracking-[0.02em]"
          :class="f.risky ? 'bg-warn-bg text-warn' : 'bg-ink text-bg'"
        >
          {{ f.ext || "file" }}
        </span>
      </div>
    </TransitionGroup>
    <span
      v-if="props.files.length > 1"
      class="num absolute end-[18%] top-1 z-10 flex h-7 min-w-7 items-center justify-center rounded-full bg-signal px-2 text-[13px] font-semibold text-white"
    >
      {{ props.files.length }}
    </span>
  </div>
</template>

<script setup lang="ts">
import { dangerous, nameParts } from "~/services/limits";

const props = defineProps<{ files: readonly { name: string }[] }>();

// Up to three cards, fanned: the first on top, in the middle
const POSE: string[][] = [
  ["z-[3]"],
  ["z-[2] -translate-x-6 -rotate-6", "z-[3] translate-x-6 rotate-3"],
  ["z-[1] -translate-x-14 -rotate-[10deg] translate-y-2", "z-[3]", "z-[2] translate-x-14 rotate-[9deg] translate-y-2"],
];
const shown = computed(() =>
  props.files.slice(0, 3).map((f, i) => ({
    key: `${i}:${f.name}`,
    ext: nameParts(f.name).ext.replace(/^\./, "").slice(0, 5),
    risky: dangerous(f.name),
  })),
);
</script>
