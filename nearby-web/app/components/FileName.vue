<!-- oeca: a received or offered file's name, never misleading: a long name is
     shortened before its extension (which always shows), and a type that runs
     code or opens something elsewhere is said to be one. -->
<template>
  <span class="flex min-w-0 items-center gap-2">
    <!-- No gap between stem and extension: a gap would read as a space in the name -->
    <span class="flex min-w-0 items-baseline overflow-hidden">
      <span class="truncate">{{ parts.stem }}</span>
      <span class="shrink-0">{{ parts.ext }}</span>
    </span>
    <span
      v-if="risky"
      class="inline-flex shrink-0 items-center gap-1 rounded-full bg-warn-bg px-2 py-[3px] text-[11px] font-semibold leading-none text-warn"
      title="This type of file can run programs or open other places. Open it only if you expected it."
    >
      <Glyph name="alert" fill class="size-3" />
      Runs programs
    </span>
  </span>
</template>

<script setup lang="ts">
import { dangerous, nameParts } from "~/services/limits";

const props = defineProps<{ name: string }>();
const parts = computed(() => nameParts(props.name));
const risky = computed(() => dangerous(props.name));
</script>
