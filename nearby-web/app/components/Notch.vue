<!-- oeca: the Oeca notch, on the phone: a black capsule that always says what
     Hop is doing (view.ts `notchState`). Its glyph changes with the mode:
     a dot (ember while Hop is on), a spinner, or a ring of confirmed
     progress. -->
<template>
  <div
    class="notch inline-flex h-10 min-w-[8.5rem] max-w-full items-center gap-2.5 rounded-full ps-3 pe-4"
    :class="{ attention: props.state.mode === 'asks' }"
    role="status"
    aria-live="polite"
  >
    <span class="relative flex size-5 shrink-0 items-center justify-center" aria-hidden="true">
      <!-- Progress: a ring (an attribute-driven dash, no inline style) -->
      <svg v-if="props.state.progress !== null" viewBox="0 0 20 20" class="size-5 -rotate-90">
        <circle cx="10" cy="10" r="7.5" fill="none" stroke="currentColor" stroke-opacity="0.22" stroke-width="2.4" />
        <circle
          cx="10"
          cy="10"
          r="7.5"
          fill="none"
          stroke="currentColor"
          :class="props.state.mode === 'done' ? 'text-good' : ''"
          stroke-width="2.4"
          stroke-linecap="round"
          :stroke-dasharray="`${Math.max(1.5, props.state.progress * 47.1)} 48`"
        />
      </svg>
      <Glyph v-else-if="props.state.mode === 'quiet'" name="arc" class="spin size-[18px] opacity-80" />
      <span v-else-if="props.state.mode === 'searching' || props.state.mode === 'nearby'" class="relative flex size-2.5">
        <span v-if="props.state.mode === 'searching'" class="live absolute inset-0 rounded-full bg-signal" />
        <span class="relative size-2.5 rounded-full bg-signal" />
      </span>
      <span v-else class="size-2 rounded-full" :class="DOT[props.state.mode] ?? 'bg-white/40'" />
    </span>
    <span class="relative min-w-0 overflow-hidden">
      <Transition name="swap">
        <span :key="props.state.label" class="num block truncate text-[14px] font-medium">{{ props.state.label }}</span>
      </Transition>
    </span>
  </div>
</template>

<script setup lang="ts">
import type { NotchMode } from "~/utils/view";

const props = defineProps<{ state: { mode: NotchMode; label: string; progress: number | null } }>();

const DOT: Partial<Record<NotchMode, string>> = {
  off: "bg-white/40",
  warn: "bg-warn",
  bad: "bg-bad",
  asks: "bg-warn",
  ended: "bg-white/60",
};
</script>
