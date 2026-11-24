<!-- oeca: a transfer's confirmed progress as one ring, the percentage in it;
     when done, the ring closes green and a check draws itself; when it
     didn't finish, the ring stops where it was, with a warning in place of the
     number (the file list says what was confirmed). -->
<template>
  <div class="relative mx-auto size-[168px]" role="progressbar" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="pct" :aria-label="props.label">
    <svg viewBox="0 0 120 120" class="size-full -rotate-90" aria-hidden="true">
      <circle cx="60" cy="60" r="52" fill="none" class="stroke-ink/[0.08]" stroke-width="8" />
      <circle
        cx="60"
        cy="60"
        r="52"
        fill="none"
        stroke-width="8"
        stroke-linecap="round"
        :stroke-dasharray="`${Math.max(3, (pct / 100) * 326.7)} 327`"
        class="transition-[stroke-dasharray] duration-500 ease-out"
        :class="props.state === 'done' ? 'stroke-good' : props.state === 'ended' ? 'stroke-ink-3' : 'stroke-ink'"
      />
    </svg>
    <div class="absolute inset-0 flex flex-col items-center justify-center">
      <svg v-if="props.state === 'done'" viewBox="0 0 24 24" class="size-14 text-good" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path class="draw" d="M5.5 12.5l4.2 4.2L18.5 7.8" />
      </svg>
      <Glyph v-else-if="props.state === 'ended'" name="alert" class="size-12 text-warn" />
      <template v-else>
        <span class="text-[40px] font-medium tabular-nums leading-none tracking-[-0.03em]">{{ pct }}<span class="text-[22px] text-ink-3">%</span></span>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{ progress: number; state: "moving" | "done" | "ended"; label: string }>();
const pct = computed(() => Math.floor(Math.min(1, Math.max(0, props.progress || 0)) * 100));
</script>
