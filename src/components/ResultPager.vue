<script setup lang="ts">
import { computed, watch } from "vue";

const props = defineProps<{
  page: number;
  total: number;
  pageSize: number;
  previousLabel: string;
  nextLabel: string;
  label: string;
}>();
const emit = defineEmits<{ "update:page": [page: number] }>();
const pageCount = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)));

// 删除或替换结果会缩短列表，及时收回页码可避免用户停留在看似丢失结果的空页。
// Removing or replacing results can shorten the list, so clamping the page prevents an empty page that looks like lost results.
watch(pageCount, (count) => {
  if (props.page > count) emit("update:page", count);
});
</script>

<template>
  <nav v-if="total > pageSize" class="result-pager" :aria-label="label">
    <button type="button" :disabled="page <= 1" @click="emit('update:page', page - 1)">{{ previousLabel }}</button>
    <span>{{ page }} / {{ pageCount }} · {{ total }}</span>
    <button type="button" :disabled="page >= pageCount" @click="emit('update:page', page + 1)">{{ nextLabel }}</button>
  </nav>
</template>
