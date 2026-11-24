<!-- Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md. -->
<!-- oeca: a sheet that floats inset from the screen (38 px corners,
     concentric with the phone's), sliding up from below; a panel in the
     middle on wide screens. Its header and actions stay put while its body
     scrolls. Modal: focus moves in (and back out), the page behind does not
     scroll. Escape and a tap outside close it only when `dismissible`. -->
<template>
  <Teleport to="body">
    <Transition name="sheet" @after-enter="focusIn">
      <div
        v-if="props.visible"
        class="fixed inset-0 z-50 flex items-end justify-center px-2 pt-10 pb-[max(0.5rem,env(safe-area-inset-bottom))] sm:items-center sm:p-6"
        @keydown.esc.stop="dismiss"
      >
        <div class="absolute inset-0 bg-black/35 backdrop-blur-[2px]" aria-hidden="true" @click="dismiss" />
        <div
          ref="panel"
          class="sheet-panel relative flex max-h-[min(88dvh,50rem)] w-full max-w-[28rem] flex-col overflow-hidden rounded-[38px] bg-sheet shadow-[0_30px_80px_-20px_rgb(0_0_0/0.45),0_0_0_1px_rgb(var(--line)/0.06)]"
          role="dialog"
          aria-modal="true"
          :aria-labelledby="props.labelledby"
          tabindex="-1"
        >
          <div class="flex shrink-0 justify-center pt-2" aria-hidden="true">
            <span class="h-[5px] w-9 rounded-full bg-ink/15 sm:invisible" />
          </div>
          <div v-if="$slots.header" class="shrink-0 px-3.5 pt-1.5 pb-2">
            <slot name="header" />
          </div>
          <div class="relative min-h-0 flex-1 overflow-y-auto overscroll-contain px-3.5 pb-4">
            <slot />
          </div>
          <div v-if="$slots.footer" class="shrink-0 px-3.5 pt-2 pb-3.5">
            <slot name="footer" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
const props = defineProps<{
  visible: boolean;
  labelledby?: string;
  dismissible?: boolean;
}>();
const emit = defineEmits<{ close: [] }>();

const panel = ref<HTMLElement | null>(null);
let before: Element | null = null;

function dismiss() {
  if (props.dismissible) emit("close");
}

function focusIn() {
  // The marked element (a heading: read first), else the first control
  const first = panel.value?.querySelector<HTMLElement>("[data-autofocus], button:not([disabled]), input, a[href]");
  (first ?? panel.value)?.focus({ preventScroll: true });
}

watch(
  () => props.visible,
  (on) => {
    if (typeof document === "undefined") return;
    // Another sheet still open (Privacy over Settings) keeps the page still
    const others = document.querySelectorAll(".sheet-panel").length > (on ? 0 : 1);
    document.documentElement.classList.toggle("overflow-hidden", on || others);
    if (on) {
      before = document.activeElement;
      // Reduced motion: no transition, so after-enter may not come
      void nextTick(() => requestAnimationFrame(focusIn));
    } else if (before instanceof HTMLElement) {
      before.focus({ preventScroll: true });
      before = null;
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (typeof document !== "undefined") document.documentElement.classList.remove("overflow-hidden");
});
</script>
