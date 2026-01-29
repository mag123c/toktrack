# Node.js CLI의 한계를 느끼고 Rust로 재작성한 이야기

> ccusage 43초 → toktrack 3초, 15배 빠른 AI 토큰 트래커를 만들기까지

---

## 개발 동기

### 갑자기 느려진 ccusage

[ccusage](https://github.com/ryoppippi/ccusage)를 출시 이래로 정말 잘 사용하고 있던 사용자 중 한 명이었습니다. Claude Code의 토큰 사용량을 한눈에 볼 수 있어서 매일같이 쓰고 있었어요.

그런데 최근 체감될 정도로 느려졌습니다. 43초. 무언가 바뀐 건 제 사용량이었어요.

```bash
# 용량
du -sh ~/.claude/projects
3.0G

# 파일 수
find ~/.claude/projects -name "*.jsonl" | wc -l
2227
```

Claude Code는 기본적으로 30일 지난 세션 파일을 자동 삭제합니다. 그럼에도 불구하고 최근 역대급으로 많이 사용해서 그런지, 세션 파일이 2,227개, 3GB나 남아있더라구요.

### 해결되지 않는 이슈들

GitHub 이슈를 확인해보니 저만의 문제가 아니었습니다.

| 이슈 | 내용 |
|------|------|
| [#821](https://github.com/ryoppippi/ccusage/issues/821) | 750 파일 / 4GB에서 timeout (30초+) |
| [#804](https://github.com/ryoppippi/ccusage/issues/804) | CPU 300%+, 메모리 2.4GB 사용 |
| [#718](https://github.com/ryoppippi/ccusage/issues/718) | "갑자기 몇 분씩 걸린다" |

성능 관련 이슈들이 열려있고, 관련 PR들도 머지되지 않은 채 방치되어 있었습니다.

직접 기여해보려고 캐시 최적화 코드를 작성하고 벤치마크를 돌려봤지만 의미 있는 개선이 없었습니다. 왜 그랬을까요?

---

## 왜 Node.js로는 해결이 어려웠을까?

ccusage는 TypeScript로 작성되어 있습니다. 먼저 기존 구현을 살펴봤습니다.

### ccusage의 구현 방식

```typescript
// ccusage의 파일 처리 방식 (간략화)
import { readFile } from 'node:fs/promises';
import { glob } from 'tinyglobby';

const files = await glob(['**/*.jsonl']);

for (const file of files) {
  const content = await readFile(file, 'utf-8');
  for (const line of content.split('\n')) {
    const parsed = JSON.parse(line);
    // 처리...
  }
}
```

`tinyglobby`로 파일을 탐색하고, `fs.readFile`로 파일 전체를 메모리에 로드한 뒤, `JSON.parse`로 한 줄씩 파싱합니다.

이 구조에서 병목이 되는 지점들을 하나씩 짚어보겠습니다.

### 1. JSON.parse의 한계

V8 엔진은 JSON 처리 성능을 꾸준히 개선해왔습니다.

- **V8 v7.6 (2019)**: JSON.parse 메모리 최적화 — 프로퍼티를 버퍼링 후 최적의 방식으로 객체 할당
- **V8 v13.8 (2025)**: [JSON.stringify SIMD 최적화](https://v8.dev/blog/json-stringify) — SIMD 명령어로 이스케이프 문자를 병렬 탐색

하지만 이러한 개선에도 불구하고, **JSON.parse는 여전히 바이트 단위의 순차 처리**입니다.

<!-- V8 JSON.parse 파이프라인 다이어그램 삽입: https://v8.dev/blog/v8-release-76 의 파서 구조 참고 -->

V8의 JSON.parse는 입력 문자열을 한 바이트씩 스캔하면서, 룩업 테이블로 토큰(`{`, `}`, `"`, `:` 등)을 분류하고, 반복적으로 값을 생성합니다. v7.6에서 재귀 파서를 반복 파서로 교체하고 메모리 할당을 최적화했지만, **한 번에 한 바이트씩 처리하는 근본 구조는 동일**합니다.

반면 [simdjson](https://simdjson.org/)은 SIMD(Single Instruction, Multiple Data) 명령어를 활용해 **한 번에 32~64바이트를 병렬 처리**합니다.

```
JSON.parse:  [a][b][c][d] ... [e][f][g][h] → 8번 연산 (순차)
simdjson:    [a,b,c,d,e,f,g,h]             → 1번 연산 (SIMD)
```

<!-- simdjson 2-stage 파이프라인 다이어그램 삽입: https://deepwiki.com/simdjson/simdjson 의 Stage 1/Stage 2 아키텍처 그림 참고 -->

simdjson은 두 단계로 나뉩니다:

1. **Stage 1 (Structural Discovery)**: SIMD로 64바이트 청크를 한 번에 스캔해 구조 문자(`{`, `[`, `:`, `,`)의 위치를 비트마스크로 추출
2. **Stage 2 (Value Materialization)**: 발견된 구조를 기반으로 실제 값을 생성

이 분리 덕분에 Stage 1은 분기 없이(branchless) 실행되어 CPU 파이프라인을 최대한 활용하고, 초당 **기가바이트 단위**의 JSON을 처리할 수 있습니다.

> 참고: simdjson은 [Node.js 바인딩](https://github.com/luizperes/simdjson_nodejs)도 존재하지만, native addon 설치 이슈와 마지막 업데이트가 5년 전이라는 유지보수 문제가 있습니다. 그리고 JSON 파싱만 빨라져도 아래에서 다룰 싱글스레드, GC 오버헤드 문제는 여전히 남아있습니다.

### 2. fs.readFile과 메모리

```typescript
const content = await readFile(filePath, 'utf-8');
```

`readFile`은 파일 전체를 한 번에 메모리에 로드합니다. 2,227개 파일을 순차적으로 읽으면서 각 파일의 내용이 메모리에 올라가고, UTF-8 디코딩이 수행되고, 이후 `split('\n')`으로 또다시 새로운 문자열 배열이 생성됩니다.

`createReadStream`으로 스트리밍 읽기가 가능하지만, 어차피 `JSON.parse`는 완전한 문자열 단위로 동작하기 때문에 근본적 해결은 아닙니다.

### 3. libuv 스레드풀 제한

Node.js의 파일 I/O는 이벤트 루프가 아닌 libuv의 스레드풀에서 처리됩니다.

<!-- libuv 아키텍처 다이어그램 삽입: https://docs.libuv.org/en/v1.x/design.html 의 이벤트 루프 다이어그램 참고 -->

```
Node.js I/O 구조:
┌──────────────────┐
│  Event Loop      │ ← 싱글스레드 (JS 실행 + 콜백 처리)
└────────┬─────────┘
         │
┌────────▼─────────┐
│  libuv 스레드풀   │ ← 기본 4스레드 (fs 작업 처리)
│  [1] [2] [3] [4] │
└──────────────────┘
```

기본값은 **4개**입니다. `UV_THREADPOOL_SIZE` 환경변수로 늘릴 수 있지만, 실제로 테스트해본 결과:

| UV_THREADPOOL_SIZE | 시간 | CPU 사용률 | 개선 |
|---|---|---|---|
| 4 (기본) | 43.4초 | 76% | - |
| 64 | 42.2초 | 80% | ~3% |
| 128 | 33.6초 | 100% | **~22%** |

스레드풀을 **32배로 늘려도 22% 개선이 한계**였습니다. CPU 사용률이 100%에 도달했는데도 33초. 파일 I/O를 병렬화해도 **JSON.parse를 실행하는 메인 스레드가 병목**이기 때문입니다.

Worker Threads로 JSON 파싱 자체를 병렬화할 수 있지만, Worker 간 데이터 전달 시 직렬화/역직렬화 오버헤드가 발생합니다. 결국 "병렬로 파싱하려면 직렬화가 필요하고, 직렬화 자체가 파싱만큼 비싸다"는 딜레마에 빠집니다.

### 4. V8 Garbage Collection

3GB 분량의 JSONL을 파싱하면 수많은 임시 객체가 생성됩니다. V8의 GC가 이를 정리하는 동안 JavaScript 실행이 멈추는 **Stop-the-world pause**가 발생합니다.

<!-- V8 GC 구조 다이어그램 삽입: https://v8.dev/blog/trash-talk 의 Generational GC / Orinoco 다이어그램 참고 -->

실제로 ccusage 실행 중 `--trace-gc` 옵션으로 GC를 트레이싱해봤습니다:

```
# GC 트레이싱 결과 (3GB / 2,227 파일 처리)
총 GC 이벤트:       504회
힙 메모리 최대:      378MB
Major GC 단일 pause: 최대 135ms
```

```
# Major GC (Mark-Compact) 로그 일부
605 ms: Mark-Compact 195.3 → 163.4 MB, 135.79ms pause
1051 ms: Mark-Compact 273.0 → 239.9 MB
1378 ms: Mark-Compact 378.9 → 225.5 MB
1650 ms: Mark-Compact 390.2 → 210.9 MB
1886 ms: Mark-Compact 375.3 → 161.7 MB
```

<!-- V8 힙 메모리 구조 다이어그램 삽입: https://deepu.tech/memory-management-in-v8/ 의 Young/Old Generation 그림 참고 -->

V8은 Orinoco GC를 통해 Incremental Marking, Concurrent Sweeping 등으로 pause를 줄이고 있지만, 대용량 데이터를 처리할 때는 여전히 무시할 수 없는 오버헤드입니다. 50초 실행 중 **504번의 GC**가 발생했다는 것은, 평균 100ms마다 한 번씩 GC가 개입한다는 뜻이니까요.

### 정리

| 병목 | 시도 | 결과 |
|------|------|------|
| JSON.parse 순차 처리 | simdjson Node.js 바인딩 | 유지보수 중단, 근본 해결 안 됨 |
| libuv 4스레드 | UV_THREADPOOL_SIZE=128 | 22% 개선 한계 |
| 메인 스레드 병목 | Worker Threads | 직렬화 오버헤드 |
| GC 오버헤드 | - | 504회 GC, 최대 135ms pause |
| **종합** | **캐시 최적화 시도** | **여전히 43초** |

Node.js의 단일 지점이 아니라, **여러 구조적 한계가 복합적으로 작용**하고 있었습니다.

---

## 왜 Rust였을까?

"그냥 다른 언어로 재작성하면 되는 거 아닌가요?" 맞는 말이지만, 어떤 언어를 선택하느냐에 따라 해결할 수 있는 문제의 범위가 달라집니다.

| 언어 | GC | SIMD | 네이티브 스레드 | cross-compile | npm 배포 |
|------|-----|------|---------------|---------------|----------|
| **Go** | ✅ 있음 | 제한적 | ✅ goroutine | ✅ | 별도 래퍼 |
| **C++** | ❌ 없음 | ✅ | ✅ | 복잡 | 복잡 |
| **Rust** | ❌ 없음 | ✅ simd-json | ✅ rayon | ✅ cargo | ✅ 쉬움 |

- **Go**: GC가 있어서 대용량 처리 시 같은 문제가 재발할 수 있음
- **C++**: 메모리 안전성 없이 대용량 파일을 다루는 건 리스크
- **Rust**: GC 없음 + 메모리 안전 보장 + simd-json/rayon 생태계 + `cargo`로 5개 플랫폼 cross-compile + npm 배포까지 한 번에

그리고 솔직히, 성능 좋기로 소문난 Rust를 이 기회에 제대로 배워보고 싶었습니다.

---

## Rust로 재작성하기

### 기술 스택

| 항목 | ccusage (Node.js) | toktrack (Rust) |
|------|-------------------|-----------------|
| JSON 파싱 | JSON.parse (순차) | simd-json (SIMD) |
| 파일 탐색 | tinyglobby | glob |
| 병렬 처리 | libuv 4스레드 | rayon (전체 코어) |
| 메모리 관리 | V8 GC | 소유권 시스템, zero-copy |
| TUI | - | ratatui |

### simd-json

```rust
// toktrack의 JSON 파싱 (zero-copy)
let data: ClaudeJsonLine = simd_json::from_slice(&mut line_bytes)?;
```

Rust의 [simd-json](https://github.com/simd-lite/simd-json)은 simdjson의 Rust 포트입니다. `from_slice`에 `&mut [u8]`을 전달하면 in-place로 파싱하여 불필요한 메모리 할당 없이 데이터를 추출합니다. 이것이 zero-copy 파싱입니다.

### rayon

```rust
// 병렬 파일 처리: .iter() → .par_iter() 한 줄 변경
let entries: Vec<UsageEntry> = files
    .par_iter()
    .flat_map(|f| parse_file(f))
    .collect();
```

[rayon](https://github.com/rayon-rs/rayon)은 데이터 병렬 처리 라이브러리입니다. `.iter()`를 `.par_iter()`로 바꾸는 것만으로 CPU 전체 코어를 활용한 병렬 처리가 가능합니다. work-stealing 알고리즘으로 코어 간 부하를 자동으로 분산합니다.

Node.js에서는 "병렬로 파싱하려면 직렬화가 필요하다"는 딜레마가 있었지만, Rust에서는 각 스레드가 독립적으로 파일을 읽고 파싱한 뒤, 결과만 모으면 됩니다. 직렬화 오버헤드가 없습니다.

---

## 결과

```bash
# ccusage (Node.js)
time ccusage daily --offline
# 43.26s

# toktrack (Rust)
time toktrack stats
# 2.98s
```

| 도구 | 시간 (2,227 파일 / 3GB) | 비교 |
|------|-------------------------|------|
| ccusage (Node.js) | ~43초 | 1x |
| **toktrack (Rust)** | **~3초** | **15x faster** |

---

## 마무리

Node.js로 해결하려고 캐시 최적화까지 시도해봤지만, 구조적 한계가 있었습니다.

- JSON.parse의 순차 처리
- libuv 스레드풀 확장해도 22% 한계
- 50초 동안 504번의 GC

이 문제들을 Rust의 simd-json과 rayon으로 해결해 15배 빠른 성능을 달성했습니다.

AI 시대에 Claude Code 같은 도구를 활용하면, 새로운 언어를 배우는 장벽이 많이 낮아졌다고 느꼈습니다. 개념만 잡고 있으면 구현은 빠르게 할 수 있으니까요. 이번 프로젝트가 Rust를 배우는 좋은 기회가 되었습니다.

toktrack은 오픈소스로 공개되어 있습니다. 피드백이나 기여는 언제든 환영합니다!

- **GitHub**: [github.com/mag123c/toktrack](https://github.com/mag123c/toktrack)
- **설치**: `npx toktrack` 또는 `cargo install toktrack`

---

## 참고 자료

- [V8 v7.6 Release — JSON.parse 개선](https://v8.dev/blog/v8-release-76)
- [V8 JSON.stringify SIMD 최적화](https://v8.dev/blog/json-stringify)
- [Trash Talk: V8 Orinoco GC](https://v8.dev/blog/trash-talk)
- [V8 Concurrent Marking](https://v8.dev/blog/concurrent-marking)
- [Visualizing Memory Management in V8](https://deepu.tech/memory-management-in-v8/)
- [libuv Design Overview](https://docs.libuv.org/en/v1.x/design.html)
- [simdjson — Parsing Gigabytes of JSON per Second](https://simdjson.org/)
- [simdjson DeepWiki (아키텍처 다이어그램)](https://deepwiki.com/simdjson/simdjson)
- [simdjson 논문 (arxiv)](https://arxiv.org/abs/1902.08318)
- [State of Node.js Performance 2024](https://nodesource.com/blog/State-of-Nodejs-Performance-2024)
- [ccusage GitHub Issues](https://github.com/ryoppippi/ccusage/issues)
