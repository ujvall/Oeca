<template>
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
</template>

<script setup lang="ts">
import { useColorMode } from "@vueuse/core";

// oeca: the status bar takes the page's own top color, for the theme chosen
// in Settings as well as the system's (the metas in nuxt.config.ts)
const mode = useColorMode({ emitAuto: true, disableTransition: false });
const BAR = { light: "#edeef1", dark: "#000000" } as const;

function sync() {
  for (const meta of document.querySelectorAll('meta[name="theme-color"]')) {
    const system = meta.getAttribute("media")?.includes("dark") ? BAR.dark : BAR.light;
    meta.setAttribute("content", mode.value === "light" || mode.value === "dark" ? BAR[mode.value] : system);
  }
}
// After the head is on the page (before that, its metas aren't there yet)
onMounted(() => watch(mode, () => requestAnimationFrame(sync), { immediate: true }));
</script>
