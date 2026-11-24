<!-- oeca: the whole privacy notice, from the first screen. -->
<template>
  <Dialog :visible="props.visible" labelledby="privacy-title" dismissible @close="emit('close')">
    <template #header>
      <SheetHeader id="privacy-title" title="What Hop shares" subtitle="Only after you start it">
        <template #end>
          <button type="button" class="round" aria-label="Close" @click="emit('close')"><Glyph name="close" class="size-5" /></button>
        </template>
      </SheetHeader>
    </template>
    <PrivacyNotice :alias="props.alias" class="pt-1" />
  </Dialog>
</template>

<script setup lang="ts">
const props = defineProps<{ visible: boolean; alias: string }>();
const emit = defineEmits<{ close: [] }>();
</script>
