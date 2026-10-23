<script setup lang="ts">
import type { TaskProgressEvent } from "../types";

defineProps<{
  progress: TaskProgressEvent;
  percent: number;
  title: string;
  status: string;
  message: string;
  filesLabel: string;
  succeededLabel: string;
  failedLabel: string;
  unitLabel: string;
}>();
</script>

<template>
  <div class="progress-card" :class="progress.outcome">
    <div class="progress-top"><strong>{{ title }}</strong><span>{{ percent }}%</span></div>
    <div class="progress-track" role="progressbar" :aria-label="title" :aria-valuenow="percent" :aria-valuemin="0" :aria-valuemax="100">
      <div class="progress-fill" :style="{ transform: `scaleX(${percent / 100})` }"></div>
    </div>
    <p class="progress-phase" role="status">{{ status }}</p>
    <p class="progress-meta">
      <span v-if="progress.currentPath" :title="progress.currentPath">{{ progress.currentPath.split(/[\\/]/).pop() }}</span>
      <span v-if="progress.total">{{ filesLabel }} {{ progress.current }}/{{ progress.total }}</span>
      <span v-if="progress.filesSucceeded != null">{{ succeededLabel }} {{ progress.filesSucceeded }}</span>
      <span v-if="progress.filesFailed != null">{{ failedLabel }} {{ progress.filesFailed }}</span>
    </p>
    <details v-if="progress.unitTotal || message" class="progress-details">
      <summary>{{ message || unitLabel }}</summary>
      <small v-if="progress.unitTotal">{{ unitLabel }} {{ progress.unitCurrent }}/{{ progress.unitTotal }} {{ progress.unitLabel }}</small>
    </details>
  </div>
</template>
