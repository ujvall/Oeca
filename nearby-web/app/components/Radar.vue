<!-- oeca: waiting on the other device: a black Oeca disc with the signal, and
     rings that leave it while the wait lasts. -->
<template>
  <div class="relative flex flex-col items-center">
    <div class="relative flex size-[132px] items-center justify-center">
      <template v-if="props.searching">
        <span class="ring-out absolute inset-[22px] rounded-full border border-signal/40" aria-hidden="true" />
        <span class="ring-out ring-out-2 absolute inset-[22px] rounded-full border border-signal/40" aria-hidden="true" />
        <span class="ring-out ring-out-3 absolute inset-[22px] rounded-full border border-signal/40" aria-hidden="true" />
      </template>
      <span class="absolute inset-0 rounded-full border border-line/[0.07]" aria-hidden="true" />
      <span class="absolute inset-[18px] rounded-full border border-line/[0.09]" aria-hidden="true" />
      <span class="notch relative flex size-[64px] items-center justify-center rounded-full shadow-[inset_0_1px_0_rgb(255_255_255/0.14),0_0_0_1px_rgb(var(--rim)/0.14)]">
        <svg viewBox="0 0 24 24" class="size-8" fill="none" stroke-linecap="round" stroke-width="2.1" aria-hidden="true">
          <path d="M7.6 12.6a6.2 6.2 0 0 1 8.8 0" class="stroke-signal" :class="{ breathe: props.searching }" />
          <path d="M4.6 9.4a10.4 10.4 0 0 1 14.8 0" class="stroke-signal" :class="{ 'breathe breathe-2': props.searching }" />
          <circle cx="12" cy="16.6" r="1.9" fill="#fff" />
        </svg>
      </span>
    </div>
    <slot />
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{ searching: boolean }>();
</script>
