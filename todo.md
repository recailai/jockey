# TODO

- [ ] macOS 原生窗口：启用 `titleBarStyle: "Overlay"`、`hiddenTitle: true`、`trafficLightPosition`，并同步前端拖拽区与安全区布局。
- [ ] 评估是否启用 `transparent + windowEffects`（毛玻璃材质）；若启用，补齐 `macOSPrivateApi` 风险策略。
- [ ] `backgroundThrottling` 调整为性能优先策略，确保长流程后台状态不中断。
- [ ] 实现运行态原生控制入口：`Start / Pause / Resume / Stop`（当前主要依赖 chat command）。
- [ ] 高风险 chat set 命令增加二次确认（删除 team、重置 workflow、停止运行流）。
- [ ] 历史时间线与实时流做虚拟列表，降低长会话渲染开销。
- [ ] 真实 ACP 进程适配层（Claude/Gemini）替换当前 Mock transport。
- [ ] 会话恢复增强：应用重启后恢复“进行中”流程与 UI 选中态。
