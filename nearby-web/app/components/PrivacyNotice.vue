<!-- oeca: the privacy notice (view.ts `privacyNotice`, tested against what the
     code does), as one group of short sections, each with its glyph. -->
<template>
  <ul class="list-group">
    <li v-for="(part, i) in privacyNotice(props.alias || 'this browser\'s name')" :key="part.title" class="flex gap-3.5 px-4 py-4">
      <span class="tile-icon mt-0.5" aria-hidden="true">
        <Glyph :name="ICONS[i] ?? 'info'" fill class="size-[18px]" />
      </span>
      <div class="min-w-0">
        <h3 class="display text-[16px] font-medium">{{ part.title }}</h3>
        <p class="mt-1 text-[14px] leading-[1.55] text-ink-2">{{ part.text }}</p>
      </div>
    </li>
  </ul>
</template>

<script setup lang="ts">
import { privacyNotice } from "~/utils/view";

const props = defineProps<{ alias: string }>();
const ICONS = ["nearby", "eye", "devices", "lock", "info"] as const;
</script>
