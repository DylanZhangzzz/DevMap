# 四客户端整体资源观察（未关闭验收）

使用冻结旧版 A1CF 和当前 1cc1831 / 4C91，在 legacy-process-cXQcFM 两工作区样例上读取相同公共 full-map 端点。每版一次冷读取、四客户端各一次预热及十次读取，Job 每 100 ms 采样。样例保留旧格式和已迁移 SQL；不是纯旧安装，也不是规模或正式性能人口。

新增采样只读取自有 Windows Job 的成员，保留进程句柄并检查 Job 归属，不全机枚举或终止发现的 PID。进程角色以 worker 保留的代理 PID 分类，未匹配的 devmap.exe 保持“其他 DevMap”，不能直接称为写核心。RSS 合计会重复计算共享页面；短命 helper 和采样间峰值可能漏掉。

Windows 的两个 Job 列表计数必须分别保存；不一致或成员来不及采样的行均标记不完整，不计为零占用。参考 [Microsoft Job process list](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_process_id_list)。首次 a6l5ix 因采样器把计数不一致当成致命错误而停止；Job 清空、数据保持，失败报告保留。修订仅保留不完整观测，不将其转为通过样本。

第二轮 stage1-owned-map-tJ8mwv 完成，两个程序退出正常、各 Job 清空、SQL/旧文件/备份保持。下表只取四代理同时出现且成员采样完整的行；各列峰值不要求发生在同一时刻。

| 项目 | 冻结旧版 | 当前候选 |
|---|---:|---:|
| 所有采样行 / 不完整行 | 129 / 15 | 54 / 2 |
| 完整四代理行 | 91 | 25 |
| 应用进程数采样最大值（排除 Node 测试 worker） | 23 | 32 |
| 应用 RSS 合计采样最大值（字节） | 175222784 | 241541120 |
| 应用 private bytes 合计采样最大值 | 204804096 | 63770624 |
| 四代理 RSS 合计采样最大值 | 33599488 | 51044352 |
| 四代理之外同时出现的 DevMap 数 | 0 | 1 或 5 |

helper 包含 Git 的 cmd/mingw64 两种可执行路径及 conhost。候选的 5 个额外 DevMap 出现在约 4.016 s 和 4.141 s 两个完整采样点；其他完整四代理样本为 1 个。尚未直接确认这些额外进程的命令角色与写核心归属，下一步应结合 runtime Hello 和启动/退出时序区分启动竞争与持续后台，不能宣称多个写核心，也不能将峰值从账本删除。

证据：同目录 baseline/candidate.job.json、worker.json、report.json、job-resource-analysis.json；配置 stage1-4c91-job-resources-config.json。生命周期采用明确的测试后计划清理，不能替代自然退出验收。此次合计 RSS 观察不支持“总内存已经下降”；核心独立 150 MiB 硬门槛仍沿用其专门资源证据，不能套在总和上。
