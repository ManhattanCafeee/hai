# hai

[![English](https://img.shields.io/badge/English-424242?style=for-the-badge&logo=readme)](../README.md)
[![中文](https://img.shields.io/badge/中文-EE4C2C?style=for-the-badge)](README.zh-CN.md)

一个能够记住你的对话、理解你的媒体、并随时间整理所讨论话题的 Telegram 聊天机器人。

## 功能

- **记住重要的事** — 跨对话保留个人事实、笔记和偏好
- **理解你的媒体** — 读取你发送的图片、视频和语音消息
- **整理对话** — 跟踪话题并在其演变过程中进行总结
- **用文字或语音回应** — 以贴合你的个性来回复
- **掌握最新知识** — 在你需要时获取外部信息
- **为你做事** — 按请求执行命令、生成图片
- **自我扩展** — 按需连接外部工具、遵循专门的技能

## 安装

### 前置条件

- 带 [pgvector](https://github.com/pgvector/pgvector) 扩展的 PostgreSQL
- 一个 Telegram 机器人令牌（来自 [@BotFather](https://t.me/BotFather)）
- 一个 LLM API 密钥（OpenAI、Anthropic、OpenRouter 或本地 Ollama）

### 从源码构建

构建需要 [Rust](https://rustup.rs/) 工具链（nightly，edition 2024）。

```bash
git clone https://github.com/hmeqo/hai.git
cd hai
cargo build --release
```

二进制文件位于 `target/release/hai` — 也可以直接用 `cargo run -p hai --` 运行。

### 配置

创建 `.hai/config.toml`：

```toml
[database]
url = "postgres://user:password@localhost:5432/hai"

[channel.telegram]
bot-token = "your-bot-token"
allowed-chat-ids = [123456789]

# 凭据来自环境变量 OPENROUTER_API_KEY，或写在块里：
# [providers.openrouter]
# api-key = "sk-or-v1-..."

[model-roles.default]
provider = "openrouter"
model = "anthropic/claude-3.5-sonnet"

[agent]
language = "zh"

[agent.personality]
name = "hai"

[prompt]
soul = "一个温暖、机智的助手，回答简洁清晰。"

[model-roles.embedding]
provider = "openrouter"
model = "openai/text-embedding-3-small"
dimension = 1536
```

### 初始化数据库

```bash
hai db create      # 若数据库不存在则创建
hai db migrate     # 建表、建扩展、建索引（幂等）
```

## 使用

### 命令

```bash
hai config                   # 打印已加载的配置（json/toml）
hai db create                # 创建数据库
hai db migrate               # 建表、建扩展、建索引（幂等）
hai db rebuild embeddings    # 重建向量嵌入
hai log                      # 事件日志 TUI（三栏）；--id N 打印单条事件
hai kb import <path>         # 导入文档到知识库（幂等）
hai kb list                  # 列出文档（可按 collection 过滤）
hai kb search <query>        # 在知识库中进行语义搜索
hai kb delete <id>           # 删除文档（级联删除 chunk）
hai kb reindex               # 重新分块/嵌入过期的文档
```

### 配置参考

`.hai/config.toml`（或 `~/.config/hai/config.toml`）。设置 `HAI_LOCAL_MODE=1` 可强制只使用 `.hai/`。键使用 kebab-case；除 `[model-roles.embedding]`（必填）外，所有节均可选并有合理默认值。

#### LLM 提供商

支持的提供商：`openrouter`、`openai`、`anthropic`、`google`、`deepseek`、`groq`、`ollama`、`xai`、`azureopenai`、`minimax`、`phind`、`requesty`。每个 kind 在其 base URL 上服务它支持的模型种类（chat / vision / transcription / embedding / image / speech / rerank）。

一个提供商有凭据即可用；本地服务则是"能碰到"即可用 —— 环境变量（每个 kind 一个，如 `OPENROUTER_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`、`GEMINI_API_KEY`）或块里的 `api-key`，环境变量优先。`[providers.<name>]` 块是可选的覆盖，不是存在性声明：

```toml
[providers.openai]
api-key = "sk-..."                       # 代替环境变量 OPENAI_API_KEY

[providers.my-gateway]                   # 块里声明了端点时不需要凭据
type = "openai"
base-url = "http://gw.internal/v1"
# headers = { X-Title = "hai" }          # 可选；网关/转售

[providers.local-rerank]                 # 只服务 rerank 的 provider：不写 type，没有聊天传输
base-url = "http://127.0.0.1:8080/v1/rerank"
```

`ollama` 完全不需要块：只要服务在 `http://localhost:11434/v1` 上应答，`provider = "ollama"` 就直接生效（换地址用块里的 `base-url`）。

#### 模型角色

`[model-roles.<role>]` 只声明模型与调用方式；行为配置在 `[agent.*]`。

```toml
[model-roles.embedding]           # 必填；缺失时出错（记忆/检索依赖它）
provider = "openrouter"
model = "openai/text-embedding-3-small"
dimension = 1536               # 必填；该模型的输出宽度（pgvector 列宽）

[model-roles.vision]              # 图像/视频理解（默认使用主模型）
model = "gpt-4o"

[model-roles.transcription]       # 音频→文本（默认使用主模型）
model = "gemini-3.1-flash-lite"

[model-roles.speech]            # 语音合成（设置 model 后 send_message 的 voice 种类可用）
provider = "openrouter"
model = "google/gemini-3.1-flash-tts-preview"
voice = "Zephyr"                # 必填；音色名随模型而异（alloy 属 OpenAI，Gemini 用 Zephyr/Kore/Aoede…），填错会被提供商拒绝（HTTP 400）
# speed = 1.0                   # 可选（0.25 ~ 4.0，默认 1.0）

[model-roles.image]             # 图像生成（设置 model 时挂载 generate_image）
provider = "openrouter"
model = "google/gemini-2.5-flash-image"

# [model-roles.rerank]           # 检索打分；缺省时保持融合顺序
# provider = "openrouter"     # 走该 provider 自带的 rerank 端点
# model = "cohere/rerank-4-fast"

# [model-roles.rerank]           # 自建 cross-encoder：base-url 就是完整端点
# provider = "local-rerank"
# model = "bge-reranker-v2-m3"
# base-url = "http://127.0.0.1:8080/v1/rerank"
```

#### 行为配置

```toml
[agent.multimodal.vision]    # 理解能力：开关与分析模板
# enabled = true
# image-prompt = "…"          # 可选；替换内置图像模板
# video-prompt = "…"          # 可选；替换内置视频模板

[agent.multimodal.transcription]
# enabled = true
# prompt = "…"                # 可选；替换内置音频模板

[agent.retrieval]            # 单次 rerank 请求携带的量
# candidates = 30
# candidate-chars = 768
# timeout-secs = 10
```

#### 注意力 / 调度

```toml
[agent.attention]
base-attention = 0.05       # Observe 事件的基础调度概率
window-secs = 30            # 被点名、或它自己刚说完话之后留意反馈的时长
```

#### 上下文

```toml
[agent.context]
context-seed-cap = 10       # 章节首次渲染时注入的消息数
pinned-memory-limit = 20    # 章节开头注入的常驻记忆条数上限
topic-idle-hours = 2        # 超过该空闲时间的话题标记 need-close（提示可以收口）
session-idle-timeout-secs = 600    # 距上次 run 结束多久关闭章节
steering = true             # 打断当前轮的新事件恢复该轮
compact-token-threshold = 150000   # 上下文 token 超过该值时章节收尾（0 = 禁用）
```
