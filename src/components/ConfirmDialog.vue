<script setup lang="ts">
import { confirmState, resolveConfirm } from '../store';
</script>

<template>
  <div class="mask" @click.self="resolveConfirm(false)">
    <div class="modal sm">
      <div class="modal-h">
        <h3>{{ confirmState.title }}</h3>
        <span class="x" @click="resolveConfirm(false)">✕</span>
      </div>
      <div class="modal-b">
        <p style="margin: 0; font-size: 12px; line-height: 1.75">{{ confirmState.message }}</p>
        <p
          v-if="confirmState.detail"
          class="mono"
          style="margin: 0; font-size: 10.5px; color: var(--t4); line-height: 1.7; max-height: 120px; overflow: auto"
        >
          {{ confirmState.detail }}
        </p>
      </div>
      <div class="modal-f">
        <button class="btn" @click="resolveConfirm(false)">取消</button>
        <button class="btn" :class="confirmState.danger ? 'danger' : 'primary'" @click="resolveConfirm(true)">
          {{ confirmState.okText }}
        </button>
      </div>
    </div>
  </div>
</template>
