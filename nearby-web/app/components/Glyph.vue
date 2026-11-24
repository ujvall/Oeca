<!-- oeca: an interface icon from utils/glyphs.ts (Phosphor Icons, MIT): Bold,
     or Fill where one is drawn. Static SVG from the package, nothing fetched. -->
<template>
  <!-- eslint-disable-next-line vue/no-v-html -- bundled, static SVG paths -->
  <svg viewBox="0 0 256 256" width="1em" height="1em" fill="currentColor" aria-hidden="true" focusable="false" v-html="shape" />
</template>

<script setup lang="ts">
import { BOLD, FILL, type GlyphName } from "~/utils/glyphs";

const props = defineProps<{ name: GlyphName; fill?: boolean }>();
const shape = computed(() => (props.fill ? FILL[props.name] : undefined) ?? BOLD[props.name]);
</script>
