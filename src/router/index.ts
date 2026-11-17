import { createRouter, createWebHashHistory } from "vue-router";
import HomeView from "../pages/HomeView.vue";

const router = createRouter({
  // Tauri production ビルドでも動作するよう Hash ヒストリーを使用
  history: createWebHashHistory(),
  routes: [
    {
      path: "/daily-report",
      name: "daily-report",
      component: () => import("../pages/DailyReportView.vue"),
      props: route => ({ filename: typeof route.query.filename === 'string' ? route.query.filename : '' }),
    },
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
