<!-- Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md. -->
<!-- oeca: one file of a transfer: its name, size and what is known about it
     (view.ts `fileRows`: confirmed, or not, never assumed). -->
<template>
  <li class="flex gap-3 px-4 py-3">
    <span class="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full" :class="MARK[mark]" aria-hidden="true">
      <Glyph :name="mark === 'done' ? 'check' : mark === 'idle' ? 'file' : 'alert'" class="size-4" />
    </span>
    <div class="min-w-0 flex-1">
      <div class="flex items-baseline gap-3">
        <FileName :name="props.row.name" class="min-w-0 flex-1 text-[15px] font-medium" />
        <span class="shrink-0 tabular-nums text-[13px] text-ink-3">{{ props.row.size }}</span>
      </div>
      <p class="mt-0.5 text-[13px] leading-snug" :class="TEXT[props.row.tone]">{{ props.row.label }}</p>
      <ProgressBar v-if="props.row.progress !== null" :progress="props.row.progress" class="mt-2" :label="props.row.name" />
    </div>
  </li>
</template>

<script setup lang="ts">
import type { FileRow, Tone } from "~/utils/view";

const props = defineProps<{ row: FileRow }>();
const mark = computed(() => (props.row.done ? "done" : props.row.tone === "bad" ? "bad" : props.row.tone === "warn" ? "warn" : "idle"));
const MARK = { done: "bg-good-bg text-good", bad: "bg-bad-bg text-bad", warn: "bg-warn-bg text-warn", idle: "bg-sunk text-ink-3" } as const;
const TEXT: Record<Tone, string> = { good: "text-good", warn: "text-warn", bad: "text-bad", plain: "text-ink-2" };
</script>
