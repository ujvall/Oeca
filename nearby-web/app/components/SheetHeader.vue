<!-- oeca: a sheet's header: round buttons at the sides (or space), the title
     and a small subtitle centred between them. -->
<template>
  <div class="grid grid-cols-[44px_1fr_44px] items-center gap-2">
    <div class="flex">
      <slot name="start" />
    </div>
    <div class="min-w-0 text-center">
      <h2 :id="props.id" tabindex="-1" data-autofocus class="display truncate text-[18px] font-medium leading-tight tracking-[-0.01em] outline-none">
        {{ props.title }}
      </h2>
      <p v-if="props.subtitle" class="sub mt-0.5 truncate text-[13px]">{{ props.subtitle }}</p>
    </div>
    <div class="flex justify-end">
      <slot name="end" />
    </div>
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{ id: string; title: string; subtitle?: string }>();
</script>
