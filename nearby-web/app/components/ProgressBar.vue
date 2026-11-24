<!-- Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md. -->
<!-- oeca: the native element (accessible as a progress bar; no inline style) -->
<template>
  <progress class="meter" :class="{ 'is-good': props.good }" max="1000" :value="value" :aria-label="props.label">
    {{ Math.round(value / 10) }}%
  </progress>
</template>

<script setup lang="ts">
const props = defineProps<{
  progress: number;
  label?: string;
  good?: boolean;
}>();
const value = computed(() => Math.round(Math.min(1, Math.max(0, props.progress || 0)) * 1000));
</script>
