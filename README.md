# Sub2Viewer

macOS 菜单栏应用：监控多个标准 **Sub2API** 站点的余额、用量与健康状态。

## 功能

- **菜单栏图标 + 弹出面板**（点击切换显示）
- **多站点**并行轮询
- **普通用户**：`GET /v1/usage`（API Key）→ 余额/剩余、今日与累计用量、状态、速率窗口、订阅窗口
- **管理员**：账号密码登录 JWT → `account-availability`（失败时回退账号列表）→ 分状态统计 + 每分组正常/限速/异常
- 金额统一 **USD**
- 密钥存本地 `secrets.json`（权限 600），配置在 `~/Library/Application Support/sub2viewer`

## 开发

依赖：

- Node.js 20+
- Rust stable
- Xcode Command Line Tools

```bash
npm install
npm run tauri dev
```

打包：

```bash
npm run tauri build
```

## 使用

1. 启动后在菜单栏找到 Sub2Viewer 图标，点击打开面板。
2. 打开 **设置**：
   - **用户模式**：填写站点 Base URL + API Key
   - **管理员模式**：填写站点 Base URL + 邮箱/密码（支持 2FA）
3. 面板会按设定间隔（默认 60s）自动刷新。

### Base URL

填写站点根地址，例如：

```text
https://sub2.example.com
```

不要带 `/v1` 业务路径；应用会自行拼接 `/v1/usage` 与 `/api/v1/...`。

## 技术栈

- Tauri 2 + Rust
- Vue 3 + TypeScript
- reqwest（HTTPS）

## 隐私

- 所有请求直连你配置的 Sub2API 站点
- 不上传数据到第三方
- API Key / 密码 / JWT 仅存本机 `~/Library/Application Support/sub2viewer/secrets.json`（仅当前用户可读）
