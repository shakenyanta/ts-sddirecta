<script setup lang="ts">
// About ページ
</script>

<template>
  <div class="about">
    <h1>About</h1>
    <p>このアプリは <strong>Tauri v2 + Vite + Vue 3 + Vue Router</strong> で構築された Windows デスクトップアプリです。</p>

    <ul>
      <li>フロントエンド: Vue 3 + TypeScript</li>
      <li>ルーティング: Vue Router 4</li>
      <li>ビルドツール: Vite 6</li>
      <li>デスクトップ: Tauri 2 (Rust)</li>
    </ul>
  </div>
</template>

<style scoped>
.about {
  max-width: 600px;
  margin: 3rem auto;
}

h1 {
  font-size: 2rem;
  color: #d63651;
  margin-bottom: 1rem;
}

p {
  margin-bottom: 1rem;
  line-height: 1.6;
  color: #444;
}

ul {
  list-style: none;
  padding: 0;
}

li {
  padding: 0.4rem 0;
  color: #555;
}

li::before {
  content: "▸ ";
  color: #2d8a5e;
}
</style>
