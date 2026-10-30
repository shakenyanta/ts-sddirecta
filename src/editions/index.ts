import { computed, readonly, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

export interface EditionInfo {
  edition: 'lite' | 'pro';
  development: boolean;
  warning: string | null;
}

const state = ref<EditionInfo | null>(null);
const error = ref('');
export const editionInfo = readonly(state);
export const editionError = readonly(error);
export const isLite = computed(() => state.value?.edition !== 'pro');
export const features = computed(() => ({ mapDetails: !isLite.value }));
export const editionLabel = computed(() => {
  if (!state.value) return error.value ? '版を取得できません' : '版を確認中';
  return `${isLite.value ? 'ライト版' : 'プロ版'}${state.value.development ? '［開発］' : ''}`;
});

let pending: Promise<void> | undefined;
export function initializeEdition(): Promise<void> {
  return pending ??= invoke<EditionInfo>('get_edition').then(info => {
    if (!info || !['lite', 'pro'].includes(info.edition) || typeof info.development !== 'boolean'
      || (info.warning !== null && typeof info.warning !== 'string')) {
      throw new Error('版の応答が不正です');
    }
    state.value = info;
  }).catch(reason => {
    error.value = `版を取得できませんでした。アプリを再起動してください: ${String(reason)}`;
  });
}
