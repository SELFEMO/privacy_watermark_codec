<script setup lang="ts">
import { computed, ref } from "vue";
import ResultPager from "./ResultPager.vue";

const props = defineProps<{
  paths: string[];
  disabled: boolean;
  title: string;
  clearLabel: string;
  removeLabel: string;
  previousLabel: string;
  nextLabel: string;
}>();
const emit = defineEmits<{ "update:paths": [paths: string[]] }>();
const page = ref(1);
const pageSize = 50;
const visiblePaths = computed(() => props.paths.slice((page.value - 1) * pageSize, page.value * pageSize));

function removePath(index: number) {
  if (props.disabled) return;
  // 按原列表索引移除而非文件名匹配，避免不同文件夹内同名媒体被一并删除。
  // Removing by the original list index instead of the filename keeps same-named media in other folders intact.
  const absoluteIndex = (page.value - 1) * pageSize + index;
  emit("update:paths", props.paths.filter((_, current) => current !== absoluteIndex));
}
</script>

<template>
  <details v-if="paths.length" class="import-summary imported-files">
    <summary>{{ title }} <span>{{ paths.length }}</span></summary>
    <div class="import-summary-head">
      <small>{{ paths.length }} {{ title }}</small>
      <button type="button" :disabled="disabled" @click="emit('update:paths', [])">{{ clearLabel }}</button>
    </div>
    <ul class="imported-file-list">
      <li v-for="(path, index) in visiblePaths" :key="path">
        <span :title="path">{{ path.split(/[\\/]/).pop() || path }}</span>
        <button type="button" :disabled="disabled" :aria-label="`${removeLabel}: ${path}`" :title="removeLabel" @click="removePath(index)">×</button>
      </li>
    </ul>
    <ResultPager v-model:page="page" :total="paths.length" :page-size="pageSize" :previous-label="previousLabel" :next-label="nextLabel" :label="title" />
  </details>
</template>
