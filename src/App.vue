<script setup lang="ts">
import { RouterView } from "vue-router";
import { initializeEdition, editionInfo, editionError } from './editions';
void initializeEdition();
</script>

<template>
  <div v-if="editionError || editionInfo?.warning" class="edition-warning" role="alert">
    {{ editionError || editionInfo?.warning }}
  </div>
  <RouterView />
</template>

<style>
.edition-warning {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  z-index: 10000;
  max-height: 40vh;
  overflow: auto;
  padding: 8px;
  background: #fff0d4;
  color: #713f12;
  font-size: 12px;
}
*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
}
</style>
