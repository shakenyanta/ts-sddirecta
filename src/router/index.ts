import { createRouter, createWebHashHistory } from "vue-router";
import HomeView from "../pages/HomeView.vue";

const router = createRouter({
  // Tauri production ビルドでも動作するよう Hash ヒストリーを使用
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      name: "home",
      component: HomeView,
    },
    {
      path: "/about",
      name: "about",
      component: () => import("../pages/AboutView.vue"),
    },
  ],
});

export default router;
