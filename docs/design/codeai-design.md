# codeai 설계 문서

Agent-first Code Exploration for Context Minimization

> 목표: **AI 에이전트가 코드 탐색 시 컨텍스트(토큰/출력량)를 최소화**
> 범위: **검색 → 후보 좁히기 → 필요한 블록만 open**
> 제외: 리팩토링/정확 rename/refactor는 **사용자 IDE/LSP**에 위임
> 구현: **Rust**, 파서는 **Tree-sitter embedded**, 검색은 **BM25 + fuzzy**

---

## 1. 문제 정의

에이전트가 코드베이스를 탐색할 때 컨텍스트를 낭비하는 패턴:

- 파일 전체를 읽어 구조를 파악함
- 함수/심볼명을 정확히 모르거나 기억이 애매하면 탐색 실패 또는 반복 호출
- 관련 문맥(상위 함수/내부 호출 함수)을 보기 위해 더 많은 코드를 읽음
- 코드 변경이 잦아 인덱스/검색 결과가 쉽게 stale 됨
- 에러 응답이 비정형이면 에이전트가 복구 액션을 결정하지 못하고 재시도를 반복함

---

## 2. 목표 / 비목표

### 목표

- **Search-before-read**: "이름 몰라도" 검색으로 블록 후보를 좁힘
- **Addressable open**: 파일 전체가 아니라 **블록(함수/클래스) 또는 range만** 읽음
- **Batch open**: 여러 블록을 한 번의 호출로 읽어 라운드트립 절감
- **Predictable output**: 항상 **budget/paging/truncation**으로 출력 폭주 방지
- **Fast sync**: 빈번한 변경에도 **증분 인덱싱**으로 최신 상태 유지
- **Monorepo + Multi-language**: 언어가 섞인 레포에서도 동일 워크플로우로 탐색
- **Agent-first format**: 사람 가독성보다 **에이전트가 파싱/후속 액션하기 쉬운 포맷** 우선
- **Structured errors**: 에러 응답도 정형 포맷으로 제공하여 에이전트의 자동 복구 지원

### 비목표

- 정확한 definition/references/rename/refactor 제공
- 통계/복잡도/impact 분석(핫스팟 등)

---

## 3. 핵심 아이디어 (markdownai 패턴의 코드 버전)

- 문서에서 `toc → read(section)` 하듯이,
- 코드에서는 `outline → open(block)`을 기본 루프로 삼는다.

핵심 루프:

1. `search`로 후보 블록을 좁힌다 (이름 몰라도 가능)
2. `open`으로 해당 블록만 읽는다 (range 기반, 배치 가능)
3. 필요하면 `open --with=callees`로 "제한된 문맥 확장"
4. 부족하면 `search` 질의를 refine

### search vs outline 사용 가이드

| 상황 | 권장 커맨드 | 이유 |
|------|------------|------|
| 파일 경로를 모르거나 키워드로 탐색 | `search` | 전체 인덱스에서 BM25+fuzzy로 후보 발견 |
| 파일 경로를 이미 알고 구조 파악 | `outline` | 해당 파일의 블록 목록을 빠르게 나열 |
| 특정 디렉토리 내 탐색 | `search --path <dir>` | path 필터로 범위 제한 |
| 파일을 열었는데 너무 크거나 복잡 | `outline` → `open` | 구조 파악 후 필요 블록만 open |

---

## 4. 아키텍처

```
┌───────────────────────────────┐
│             CLI               │
│  index / search / outline     │
│  open / watch                 │
└───────────────┬───────────────┘
                │  (Agent-first Output)
                ▼
┌───────────────────────────────┐
│     Query / Output Layer      │
│  - ranking / filtering        │
│  - cursor paging              │
│  - truncation & byte budget   │
│  - error formatting           │
└───────────────┬───────────────┘
                │
        ┌───────┴───────┐
        ▼               ▼
┌──────────────┐   ┌─────────────────┐
│ SQLite (meta)│   │ Search Index     │
│ files/blocks │   │ BM25 + fuzzy     │
└──────┬───────┘   └──────┬──────────┘
       │                  │
       └────────┬─────────┘
                ▼
┌───────────────────────────────┐
│       Parser / Extract        │
│       Tree-sitter             │
│       (embedded SDK)          │
└───────────────────────────────┘

Source of truth: repo files.
DB/Index: cache for fast, minimal reads.
```

### 4.1 인덱스 저장 위치 및 라이프사이클

**저장 경로**: `.worktoolai/codeai/` 디렉토리 (프로젝트 루트)

```
.worktoolai/codeai/
├── index.db          # SQLite (블록 메타, 파일 메타)
├── search/           # Tantivy 검색 인덱스
├── ignore            # 사용자 제외 패턴 (선택)
└── lock              # 인덱싱 락 파일
```

**락 정책**:
- `index` 실행 시 `.worktoolai/codeai/lock` 파일로 배타적 락 획득
- 락 보유 중 다른 `index` 호출 → `INDEX_BUSY` 에러 반환
- 비정상 종료 시 stale 락 감지: 락 파일 내 PID 확인 → 프로세스 부재 시 자동 해제

**정리 정책**:
- `codeai index --full`: 기존 인덱스 삭제 후 전체 재구축
- `.worktoolai/codeai/` 삭제 후 `codeai index`: 완전 초기 상태에서 재구축 (안전)

**브랜치 전환**: 인덱스는 브랜치를 구분하지 않는다. 파일 변경 기반 증분 인덱싱이므로, 브랜치 전환 후 `codeai index`를 실행하면 변경된 파일만 자동 갱신된다.

---

## 5. 데이터 모델 (Block-as-Document)

### 5.1 최소 단위: Block

인덱싱/검색/오픈의 "문서 단위"는 파일이 아니라 **블록**이다.

- function / method / class / struct / interface / module-level block

### 5.2 블록 필드 (MVP)

- `symbol_id`: 안정적인 식별자 (아래 5.3 참고)
- `language`: 파일 단위 판별 결과
- `kind`: function|method|class|...
- `name`: 심볼명 (없으면 `<anon>` placeholder)
- `path`
- `range`: start_line, start_col, end_line, end_col (바이트 오프셋 아님, 0-based 라인/컬럼)
- `signature` (가능한 범위, 언어별 추출 규칙은 §12.7 참고)
- `doc` (언어별 docstring/주석 추출 규칙은 §12.7 참고)
- `preview`: 본문 앞 N줄 (기본 10~30)
- `strings`(옵션): string literals(로그/에러/이벤트명) 추출

### 5.3 symbol_id 설계

**문제**: `hash(path + kind + name + range)` 방식은 주석 한 줄 추가만으로 range가 바뀌어 ID가 불안정해진다. 에이전트가 이전 턴에서 받은 symbol_id로 `open`을 호출하면 실패할 수 있다.

**설계**:

- **ID 포맷**: 사람이 읽을 수 있고 역참조 가능한 구조화된 문자열
  ```
  <normalized_path>#<kind>#<name>
  <normalized_path>#<kind>#<name>#<occurrence_index>
  ```
  예시: `services/payment/validate.go#func#ValidatePayment`, `utils/helpers.ts#func#parse#1`

- **경로 정규화**: OS별 경로 구분자 차이를 방지하기 위해 항상 `/` 사용, 프로젝트 루트 기준 상대 경로
- **동명 심볼 구분**: 같은 파일에 같은 kind+name이 1개뿐이면 `#occurrence_index`를 생략. 여러 개 존재할 경우에만 출현 순서 인덱스(0-based)를 추가
- **occurrence_index 안정성 한계**: 동명 함수 사이에 새 동명 함수가 삽입/삭제되면 인덱스가 밀린다. 이는 알려진 한계이며, 실제로 같은 파일에 같은 이름의 함수가 여러 개 존재하는 경우는 드물다
- **내부 lookup 최적화**: DB 내부에서는 symbol_id 문자열의 xxh3 해시를 인덱스 키로 사용하여 검색 성능 확보

**stale ID fallback**:
- `open`에서 symbol_id가 DB에 없을 경우, ID 문자열에서 path/kind/name을 직접 파싱하여 현재 인덱스에서 재탐색
- 후보가 1개면 자동 매칭 후 응답에 `resolved_from` 필드로 원래 ID → 새 ID 매핑을 포함
- 후보가 0개면 `SYMBOL_NOT_FOUND` 에러 + search recovery 힌트
- 후보가 여러 개면 `SYMBOL_AMBIGUOUS` 에러 + 후보 목록을 에러 응답으로 반환

이렇게 하면 함수 본문이나 주변 코드가 변경되어도, 함수 자체가 삭제/이름변경되지 않는 한 ID가 유지된다.

### 5.4 익명 함수/람다 처리

이름이 없는 블록(익명 함수, 람다 등)의 처리:

- **변수 할당된 arrow function/함수 표현식**: 변수명을 `name`으로 사용 (예: `const handler = () => {}` → name: `handler`)
- **export default**: `<default>` placeholder 사용
- **완전 익명 (콜백 등)**: `<anon>` placeholder + 출현 인덱스로 구분
- 익명 블록은 검색에서 name 필드로는 찾기 어려우므로, `signature`, `doc`, `strings` 필드의 가중치가 높아진다

---

## 6. 검색 설계 (함수명 몰라도 찾기)

### 6.1 MVP: Lexical-first Search

- BM25 + fuzzy/prefix
- 인덱싱 대상: 블록 단위 텍스트
  - name, signature, doc, path
  - strings(로그/에러 문구)
  - preview(제한적으로)

### 6.2 랭킹 힌트(저비용)

- name exact/prefix boost
- doc/signature match boost
- path match boost (`auth/`, `payment/` 등)

> "정확한 의미 분석"이 아니라, **후보 블록을 빠르게 좁히는 것**이 목표.

### 6.3 검색 품질 평가 (Eval Set)

MVP 단계부터 간단한 평가 세트를 유지하여 검색 품질을 측정한다.

**포맷**: `eval/search_cases.jsonl` — 각 줄이 하나의 테스트 케이스

```json
{
  "query": "payment validation failed",
  "expected_top5": ["services/payment/validate.go::ValidatePayment"],
  "tags": ["error-string", "cross-module"]
}
```

**평가 지표**:
- **Recall@5**: expected 블록이 상위 5개 결과에 포함되는 비율
- **MRR (Mean Reciprocal Rank)**: expected 블록의 순위 역수 평균
- **Latency p95**: 검색 응답 시간 95퍼센타일
- **Open 성공률**: search 결과의 symbol_id로 open했을 때 성공하는 비율

**실행**: `codeai eval --cases eval/search_cases.jsonl` (CLI 서브커맨드)

**활용**: 향후 임베딩 검색 도입 여부를 "Recall@5가 특정 임계값 이하로 떨어지는 쿼리 유형이 N% 이상일 때" 같은 기준으로 판단할 수 있다.

---

## 7. Open 설계 (Addressable + Context Expansion)

### 7.1 기본 open

- `open --symbol <symbol_id>`: 블록만 출력
- `open --range <path>:<L:C-L:C>`: 지정 범위만 출력

> Source of truth는 파일이며, DB는 "주소와 미리보기"만 가진다.

### 7.2 배치 open

여러 블록을 한 번에 읽어 라운드트립을 줄인다:

- `open --symbols <id1>,<id2>,<id3>`
- 각 블록은 개별 item으로 출력되며, 전체가 `--max-bytes` budget 내에서 할당
- budget 분배: 균등 분배 후 남은 여유분을 앞쪽 블록에 재할당
- budget 초과 시: 뒤쪽 블록부터 truncate하되, 처리된 블록은 정상 출력
- 미처리 블록은 `remaining` 필드에 symbol_id 목록으로 제공 (에이전트가 후속 배치 호출에 사용)
- `truncated=1`이 설정되지만, `next_cursor`는 사용하지 않음 (배치 open은 cursor 기반 페이징이 아님)

### 7.3 문맥 확장: open with lexical callees (휴리스틱)

상위 함수의 내부 호출 함수들을 한 번에 보고 싶을 때:

- `open --symbol <id> --with=callees`
- 안전 옵션(필수):
  - `--depth N` (기본 1)
  - `--max-fns K` (기본 6~10)
  - `--preview-lines M` (기본 15~25)
  - `--max-bytes B` (기본 8KB~16KB)
  - `--dedupe` (기본 on)

**구현**:

- Tree-sitter로 **call-like 패턴**(함수 호출 노드)을 추출
- 같은 파일/근접 경로/동일 모듈 우선으로 매칭
- 각 callee는 "전체 본문"이 아니라 **짧은 preview**만 포함
- 각 callee에 `confidence` 필드를 포함: `high` (같은 파일 내 유일 매칭), `medium` (같은 디렉토리/모듈), `low` (동명 함수 다수 존재)

**한계 명시**: 이 기능은 **lexical callees**(구문 수준 호출 패턴 매칭)이다. 다음은 감지하지 못한다:

- 고차함수 콜백 (`arr.map(myFunc)` 에서 `myFunc`의 정의)
- 동적 디스패치 (`obj[methodName]()`)
- 메서드 체이닝의 중간 호출
- 다른 모듈의 동명 함수 간 정확한 구분

매칭 실패 시 빈 callees 배열을 반환하며, 이는 에러가 아니라 정상 응답이다.

---

## 8. 출력 포맷 (Agent-first)

### 8.1 왜 JSON이냐 / 왜 얇은 포맷이냐

- JSON은 키/괄호로 토큰이 늘 수 있으나,
- **파싱 안정성**과 **후속 액션 결정의 정확도**가 높아
- "재시도/재질문/추가 호출"을 줄여 **총 컨텍스트 비용을 절감**할 수 있다.

따라서 기본은 **에이전트 안정성 우선 포맷**을 사용하되,
토큰 낭비를 줄이기 위해 **Thin JSON(튜플)**을 기본으로 채택한다.

### 8.2 포맷 옵션

- `--fmt thin` (기본): 튜플 기반 최소 JSON
- `--fmt json`: 가독성 있는 표준 JSON(디버그/테스트용)
- `--fmt lines`(선택): 매우 압축된 line 포맷(토큰 최소, 기계 파싱도 가능)

### 8.3 Thin JSON 스키마(권장)

#### 스키마 버전

모든 응답의 최상위에 `v` 필드를 포함하여 스키마 버전을 명시한다:

- `v`: 정수 (현재 `1`). 필드 추가/순서 변경 시 증가.
- 에이전트는 `v` 값을 확인하여 호환되지 않는 버전이면 파싱을 중단하고 사용자에게 알릴 수 있다.

**하위 호환 정책**: 튜플 끝에 필드를 추가하는 것은 `v`를 올리지 않는다 (에이전트는 알려진 인덱스까지만 읽으면 됨). 기존 필드의 의미/순서 변경 시에만 `v`를 올린다.

#### 성공 응답

공통:
- `v`: schema version (정수)
- `m`: meta (cmd, budget, truncation, cursor)
- `i`: items (튜플 배열)
- `h`: hints (다음 액션 힌트)

예시:
```json
{
  "v": 1,
  "m": ["search", 12000, 8421, 0, null],
  "i": [
    ["services/payment/validate.go#func#ValidatePayment","ValidatePayment","services/payment/validate.go","34:0-128:1",12.4,["doc","str"],"func ValidatePayment..."],
    ["auth/verify.ts#func#checkAuth","checkAuth","auth/verify.ts","10:0-60:0",11.2,["name"],"function checkAuth..."]
  ],
  "h": [["open",{"symbol_id":"services/payment/validate.go#func#ValidatePayment"}]]
}
```

meta 튜플 정의(고정):

- `m = [cmd, max_bytes, byte_count, truncated(0/1), next_cursor|null]`

items 튜플 정의(커맨드별 고정):

- search: `[symbol_id, name, path, range, score, why[], preview]`
- outline: `[symbol_id, name, kind, path, range]`
- open: `[symbol_id, name, path, range, signature?, doc?, content_or_preview]`

#### 에러 응답

에러도 동일한 Thin JSON 구조를 유지하여 에이전트가 일관되게 파싱할 수 있게 한다.

```json
{
  "v": 1,
  "m": ["open", 16000, 0, 0, null],
  "e": {
    "code": "SYMBOL_NOT_FOUND",
    "message": "symbol_id 'services/payment/validate.go#func#ValidatePayment' not found in current index",
    "recovery": [
      ["search", {"query": "ValidatePayment", "path": "services/payment/"}],
      ["index", {"path": "services/payment/"}]
    ]
  }
}
```

**에러 필드 정의**:
- `e.code`: 정형화된 에러 코드 (아래 표 참고)
- `e.message`: 사람 가독성 메시지
- `e.recovery`: 에이전트가 시도할 수 있는 복구 액션 힌트 (선택)

**에러 코드 목록 (MVP)**:

| code | 상황 | 권장 recovery |
|------|------|--------------|
| `SYMBOL_NOT_FOUND` | symbol_id가 인덱스에 없고 fallback 재탐색도 실패 | search로 재탐색 또는 re-index |
| `SYMBOL_AMBIGUOUS` | stale fallback 시 동명 후보가 여러 개 | 후보 목록에서 선택 또는 search로 재탐색 |
| `FILE_NOT_FOUND` | path가 파일시스템에 없음 | search로 유사 경로 탐색 |
| `RANGE_OUT_OF_BOUNDS` | range가 파일 범위 초과 | outline으로 현재 구조 확인 |
| `PARSE_ERROR` | Tree-sitter 파싱 실패 | open --range로 raw 텍스트 읽기 |
| `INDEX_EMPTY` | 인덱스가 비어 있음 | index 실행 |
| `INDEX_BUSY` | 다른 인덱싱 프로세스가 실행 중 | 잠시 후 재시도 |
| `BUDGET_EXCEEDED` | 단일 블록이 max-bytes 초과 | preview-lines 줄이거나 range로 부분 읽기 |
| `CURSOR_STALE` | cursor가 참조하는 인덱스 세대가 현재와 불일치 | cursor 없이 처음부터 재조회 |
| `UNSUPPORTED_LANGUAGE` | 파일 확장자가 지원 언어 목록에 없음 | open --range로 raw 텍스트 읽기 |
| `ENCODING_ERROR` | 파일이 유효한 UTF-8이 아님 | open --range로 lossy 읽기 시도 |
| `PERMISSION_DENIED` | 파일 읽기 권한 없음 | 권한 확인 후 재시도 |

### 8.4 Budget / Paging 규칙

- 모든 커맨드는 `--max-bytes`를 기본 적용 (기본값 제공)
- 초과 시:
  - truncated=1
  - next_cursor 제공
  - JSON은 항상 완전한 구조 유지(중간 깨짐 금지)

**cursor 수명 규약**:
- cursor는 내부에 인덱스 세대 번호(`generation`)를 포함한다
- 인덱스가 갱신되면 세대 번호가 증가한다
- cursor의 세대와 현재 인덱스 세대가 불일치하면 `CURSOR_STALE` 에러를 반환한다
- 에이전트는 cursor 없이 처음부터 재조회해야 한다 (부분 결과 중복/누락 방지)

**배치 open과 cursor의 구분**:
- `search`, `outline`: cursor 기반 페이징 사용
- `open --symbols` (배치): cursor 미사용. budget 초과 시 `remaining` 필드로 미처리 ID 목록 제공

### 8.5 텍스트 인코딩 / 오프셋 규약

- **라인/컬럼**: 0-based. 라인은 `\n` 기준 분리. 컬럼은 **바이트 오프셋** (UTF-8 멀티바이트 문자의 경우 1 문자가 여러 컬럼 차지)
- **CRLF 처리**: 입력 시 `\r\n` → `\n`으로 정규화하여 처리. 출력 시에는 원본 파일의 줄바꿈을 보존
- **UTF-8 안전**: 파일 읽기 시 `from_utf8_lossy` 사용. 유효하지 않은 바이트는 U+FFFD로 대체하되, `ENCODING_ERROR` 경고를 `m` 또는 별도 필드로 포함
- **truncation 안전**: 바이트 budget으로 자를 때 UTF-8 코드포인트 중간에서 절단하지 않는다. 코드포인트 경계로 내려서 자른다

---

## 9. CLI (MVP)

### 9.1 Commands

- `codeai index [--full] [--path] [--lang] [--no-gitignore]`
- `codeai search <query> [--limit] [--path] [--lang] [--max-bytes] [--cursor] [--fmt]`
- `codeai outline <path> [--kind] [--limit] [--max-bytes] [--cursor] [--fmt]`
- `codeai open (--symbol <id> | --symbols <id1,id2,...> | --range <path:L:C-L:C>) [--preview-lines] [--with=callees ...] [--max-bytes] [--fmt]`
- `codeai eval --cases <path>` (검색 품질 평가)
- (선택) `codeai watch [--path] [--lang]`

> 참고: .gitignore는 기본적으로 존중한다. `--no-gitignore`로 비활성화할 수 있다.

### 9.2 Defaults(권장)

- `search`: limit=10, max-bytes=12KB
- `open`: preview-lines=60~120(언어/파일 크기에 따라), max-bytes=16KB
- `open --symbols` (배치): max-bytes=32KB (블록 수에 비례하여 자동 조정)
- `open --with=callees`: depth=1, max-fns=6, preview-lines=20, max-bytes=20KB

---

## 10. 파일 제외 전략

### 10.1 .gitignore 존중

기본적으로 `.gitignore`에 명시된 패턴을 인덱싱에서 제외한다. 중첩된 `.gitignore` 파일도 재귀적으로 처리한다.

### 10.2 기본 제외 패턴 (Built-in)

.gitignore가 없거나 불충분한 경우를 대비한 기본 제외 목록:

- `node_modules/`, `vendor/`, `.venv/`, `__pycache__/`
- `dist/`, `build/`, `target/`, `out/`
- `.git/`, `.svn/`, `.hg/`
- `*.min.js`, `*.min.css`, `*.map`
- 바이너리 파일 (확장자 기반: `.exe`, `.dll`, `.so`, `.dylib`, `.png`, `.jpg` 등)

### 10.3 사용자 설정

- `codeai index --ignore-file <path>`: 추가 제외 패턴 파일 지정
- `.worktoolai/codeai/ignore`: 프로젝트 루트의 커스텀 제외 패턴 (`.gitignore` 문법 동일)
- `codeai index --no-default-ignores`: 기본 제외 패턴 비활성화

### 10.4 우선순위 (높은 것이 우선)

제외 규칙이 충돌할 경우 아래 순서로 적용한다:

| 우선순위 | 소스 | 설명 |
|----------|------|------|
| 1 (최우선) | CLI 플래그 (`--ignore-file`, `--no-default-ignores`, `--no-gitignore`) | 명시적 CLI 인자가 항상 최우선 |
| 2 | `.worktoolai/codeai/ignore` | 프로젝트별 커스텀 제외 |
| 3 | `.gitignore` (중첩 포함) | 디렉토리별 .gitignore |
| 4 (최하위) | Built-in 기본 제외 | 코드에 하드코딩된 기본 패턴 |

### 10.5 구현

- `ignore` 크레이트 사용 (ripgrep과 동일한 .gitignore 파서)
- 스캔 단계에서 제외하여 불필요한 파일 I/O 자체를 방지

### 10.6 미지원 파일 처리

- **미지원 확장자**: 인덱싱에서 조용히 스킵 (경고 없음). `--lang` 필터 사용 시에만 해당 언어 파일만 인덱싱
- **비 UTF-8 파일**: 인덱싱 스킵 + 경고 로그. `open --range`로는 `from_utf8_lossy`로 읽기 가능
- **대용량 파일** (기본 1MB 초과): 인덱싱 스킵 + 경고 로그. `open --range`로는 접근 가능. `--max-file-size` 옵션으로 임계값 조정 가능
- **symlink**: 실제 파일을 따라가되, 이미 방문한 inode는 스킵하여 순환 방지

---

## 11. Sync 전략 (빈번 변경 대응)

### 11.1 변경 감지

DB에 파일 메타 저장:
- path, mtime, size, content_hash

알고리즘:
1. 빠른 스캔: mtime/size로 변경 후보
2. 확정: 해시로 실제 변경 판단

### 11.2 증분 인덱싱 (Generation 기반 원자적 업데이트)

변경된 파일에 대해:
1. Tree-sitter parse
2. 블록 추출
3. 새 generation 번호 할당
4. SQLite 트랜잭션 시작:
   - 해당 파일의 기존 blocks 삭제
   - 새 blocks 삽입
   - generation 번호 업데이트
5. Tantivy 인덱스 업데이트:
   - 해당 파일의 기존 문서 삭제
   - 새 문서 삽입
   - commit
6. SQLite에서 `current_generation`을 새 generation으로 업데이트 (최종 포인터 스왑)

**원자성 보장**: SQLite 업데이트와 Tantivy 커밋이 모두 성공한 후에만 `current_generation`을 갱신한다. Tantivy 커밋 실패 시 SQLite 트랜잭션도 롤백하여 두 저장소 간 불일치를 방지한다.

**generation 활용**: cursor에 generation 번호를 내장하여, 인덱스 갱신 후 stale cursor를 감지한다 (§8.4 참고).

### 11.3 실패/부분 정합성 처리

- 특정 파일 parse 실패해도 전체 인덱싱은 계속
- 실패 파일은 `parse_error`로 기록하고, 에러 응답 시 `PARSE_ERROR` 코드와 함께 `open --range` fallback을 안내
- `open --range`는 인덱스를 경유하지 않고 파일에서 직접 읽으므로 인덱스 stale의 영향을 받지 않는다
- `open --symbol`은 인덱스의 range 정보를 사용하므로, 인덱스가 stale이면 `RANGE_OUT_OF_BOUNDS`가 발생할 수 있다. 이 경우 `outline`으로 현재 구조를 재확인하는 recovery가 제공된다

---

## 12. Monorepo / Multi-language

### 12.1 지원 언어 (MVP: 18개)

블록 구조가 명확하여 동일한 추출 로직으로 커버되는 언어들을 하드코딩한다. 언어별 차이는 노드 이름 매핑 테이블과 이름 추출 규칙이며, 추출 코드 자체는 공통이다.

| 언어 | 확장자 | grammar crate | function 노드 | class/type 노드 |
|------|--------|---------------|---------------|-----------------|
| Go | `.go` | `tree-sitter-go` | `function_declaration`, `method_declaration` | `type_declaration` |
| Rust | `.rs` | `tree-sitter-rust` | `function_item` | `impl_item`, `struct_item`, `enum_item`, `trait_item` |
| Python | `.py` | `tree-sitter-python` | `function_definition` | `class_definition` |
| TypeScript | `.ts` | `tree-sitter-typescript` (`language_typescript()`) | `function_declaration`, `method_definition`, `arrow_function`(변수 할당) | `class_declaration`, `interface_declaration` |
| TSX | `.tsx` | `tree-sitter-typescript` (`language_tsx()`) | (TypeScript와 동일) | (TypeScript와 동일) |
| JavaScript | `.js`, `.mjs`, `.cjs` | `tree-sitter-javascript` | `function_declaration`, `method_definition`, `arrow_function`(변수 할당) | `class_declaration` |
| JSX | `.jsx` | `tree-sitter-javascript` | (JavaScript와 동일) | (JavaScript와 동일) |
| Java | `.java` | `tree-sitter-java` | `method_declaration` | `class_declaration`, `interface_declaration` |
| Kotlin | `.kt`, `.kts` | `tree-sitter-kotlin` | `function_declaration` | `class_declaration`, `object_declaration` |
| C | `.c`, `.h` | `tree-sitter-c` | `function_definition` | `struct_specifier` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx` | `tree-sitter-cpp` | `function_definition` | `class_specifier`, `namespace_definition` |
| C# | `.cs` | `tree-sitter-c-sharp` | `method_declaration` | `class_declaration`, `interface_declaration` |
| Swift | `.swift` | `tree-sitter-swift` | `function_declaration` | `class_declaration`, `protocol_declaration` |
| Scala | `.scala`, `.sc` | `tree-sitter-scala` | `function_definition` | `class_definition`, `object_definition`, `trait_definition` |
| Ruby | `.rb` | `tree-sitter-ruby` | `method` | `class`, `module` |
| PHP | `.php` | `tree-sitter-php` | `function_definition`, `method_declaration` | `class_declaration` |
| Bash | `.sh`, `.bash` | `tree-sitter-bash` | `function_definition` | (없음) |
| HCL | `.tf`, `.hcl` | `tree-sitter-hcl` | (없음) | `block` (resource, module, variable 등) |

### 12.2 LangConfig 구조 (하드코딩)

```rust
struct LangConfig {
    language: &'static str,
    extensions: &'static [&'static str],
    function_nodes: &'static [&'static str],
    class_nodes: &'static [&'static str],
    /// 노드 타입별 이름 추출 함수.
    /// 기본은 "name" child 필드이지만,
    /// 일부 노드는 부모 컨텍스트(변수 선언 등)에서 이름을 가져와야 한다.
    name_extractor: fn(node: &Node, source: &[u8]) -> Option<String>,
}

/// 기본 이름 추출: node.child_by_field_name("name")
fn default_name_extractor(node: &Node, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| n.utf8_text(source).ok().map(|s| s.to_string()))
        .flatten()
}

/// JS/TS arrow function 이름 추출:
/// 부모가 variable_declarator/assignment인 경우 변수명을 사용
fn js_arrow_name_extractor(node: &Node, source: &[u8]) -> Option<String> {
    // 1. 노드 자체의 name 필드 시도
    if let Some(name) = default_name_extractor(node, source) {
        return Some(name);
    }
    // 2. 부모가 variable_declarator면 "name" 필드에서 변수명
    if let Some(parent) = node.parent() {
        if parent.kind() == "variable_declarator" {
            return parent.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok().map(|s| s.to_string()));
        }
    }
    None
}

const GO: LangConfig = LangConfig {
    language: "go",
    extensions: &["go"],
    function_nodes: &["function_declaration", "method_declaration"],
    class_nodes: &["type_declaration"],
    name_extractor: default_name_extractor,
};

// ... 나머지 17개도 동일 패턴.
// JS/TS의 arrow_function은 js_arrow_name_extractor 사용.
```

### 12.3 grammar crate 파서 초기화

일부 언어는 하나의 crate에 여러 파서가 포함되어 있으므로 초기화 시 구분이 필요하다:

```rust
// tree-sitter-typescript 크레이트 → 2개의 language
".ts"  → tree_sitter_typescript::language_typescript()
".tsx" → tree_sitter_typescript::language_tsx()

// tree-sitter-javascript 크레이트 → JSX 내장
".js"  → tree_sitter_javascript::language()
".jsx" → tree_sitter_javascript::language()  // JSX 지원 내장
```

나머지 언어는 1 crate = 1 language로 단순하게 매핑된다.

### 12.4 언어 판별

- **1차**: 확장자 → LangConfig 매핑 테이블로 판별
- **2차(보조)**: 루트 마커(go.mod, package.json, Cargo.toml 등)로 레포 내 주 언어 힌트
- 파일별로 language 태깅

### 12.5 공용 워크플로우

- 인덱싱/검색/오픈 UX는 모든 언어에서 동일
- 언어별 정확 분석은 목표가 아니므로, **언어별 설치 의존성은 최소화**(Tree-sitter만 내장)

### 12.6 설계 원칙: 완전 내장 (Zero-config)

- 18개 언어의 grammar과 노드 매핑은 **모두 바이너리에 내장**한다
- 사용자가 언어 설정, 쿼리 파일, 플러그인을 추가할 필요가 없다
- `codeai index`만 실행하면 확장자를 보고 자동으로 적절한 파서와 블록 추출 규칙을 적용한다
- 새 언어 추가는 **codeai 자체의 코드 변경 + 릴리스**로만 이루어진다
- 빌드 최적화: 빌드 시간이 길어지면 `cargo feature`로 언어 그룹별 선택 빌드를 지원할 수 있다 (기본은 all-in-one)

### 12.7 언어별 doc/signature 추출 규칙

언어마다 docstring/주석 관례가 다르므로, 추출 규칙을 언어별로 정의한다:

| 언어 | doc 추출 규칙 | signature 추출 규칙 |
|------|---------------|---------------------|
| Python | 함수 본문 첫 `expression_statement`가 string이면 docstring | `def` 라인 전체 (데코레이터 포함) |
| Go | 함수 직전 연속 `//` comment | `func` 라인 전체 (receiver 포함) |
| Rust | 함수 직전 `///` 또는 `//!` comment, `#[doc = ...]` attribute | `fn` 라인 (제네릭, where절 포함) |
| Java/Kotlin | 함수 직전 `/** ... */` (Javadoc) | 메서드 선언 라인 (annotation 제외) |
| JS/TS | 함수 직전 `/** ... */` (JSDoc) | 함수 선언 라인 (타입 포함) |
| C/C++ | 함수 직전 `/** ... */` 또는 `///` | 함수 선언 라인 |
| 기타 | 함수 직전 연속 comment 블록 | 함수 선언 첫 줄 |

**공통 규칙**: "직전"이란 함수 노드의 바로 이전 sibling이 comment 노드인 경우를 의미한다. 빈 줄이 사이에 있으면 연결하지 않는다.

---

## 13. Rust 구현 가이드 (MVP)

### 13.1 주요 크레이트(제안)

- CLI: `clap`
- 스캔: `walkdir`
- 파일 제외: `ignore` (gitignore 호환 패턴 매칭)
- watch(옵션): `notify`
- DB: `rusqlite` (또는 `sqlx` + sqlite)
- Search: `tantivy`
- Hash: `xxhash-rust`(xxh3) 또는 `blake3`
- Tree-sitter: `tree-sitter` + 언어 grammar crate들

### 13.2 구성 원칙

- "큰 텍스트"는 DB에 전부 저장하지 않는다(프리뷰만)
- 본문은 `open` 시에 파일에서 range로 읽는다
- 출력은 thin JSON 기본 (`--fmt thin`)
- 에러 응답도 반드시 Thin JSON 구조를 유지

---

## 14. 예시 워크플로우 (Agent)

### 14.1 기본 탐색

```bash
# 1. 후보 탐색
codeai search "payment validation failed" --limit 10 --fmt thin

# 2. 후보 하나 open
codeai open --symbol "services/payment/validate.go#func#ValidatePayment" --preview-lines 80 --fmt thin

# 3. 문맥 확장(제한적)
codeai open --symbol "services/payment/validate.go#func#ValidatePayment" --with=callees --depth 1 --max-fns 6 --preview-lines 20 --max-bytes 20000 --fmt thin
```

### 14.2 배치 비교

```bash
# search 결과에서 후보 3개를 한 번에 비교
codeai open --symbols "services/payment/validate.go#func#ValidatePayment,auth/verify.ts#func#checkAuth,utils/helpers.ts#func#parse" --preview-lines 40 --max-bytes 32000 --fmt thin
```

### 14.3 파일 구조 파악 후 진입

```bash
# 파일 경로를 알 때: outline으로 구조 파악
codeai outline services/payment/validate.go --fmt thin

# 원하는 블록만 open
codeai open --symbol "services/payment/validate.go#func#ValidatePayment" --fmt thin
```

### 14.4 에러 복구

```bash
# stale symbol_id로 open 시도 → SYMBOL_NOT_FOUND 에러
codeai open --symbol "services/payment/old_validate.go#func#ValidatePayment" --fmt thin

# 에러 응답의 recovery 힌트를 따라 재탐색
codeai search "ValidatePayment" --path "services/payment/" --fmt thin

# 새 symbol_id로 open
codeai open --symbol "services/payment/validate.go#func#ValidatePayment" --fmt thin
```

### 14.5 cursor stale 처리

```bash
# 페이징 중 인덱스가 갱신된 경우
codeai search "handler" --cursor "eyJnIjozLCJvIjoxMH0=" --fmt thin
# → CURSOR_STALE 에러 반환

# cursor 없이 처음부터 재조회
codeai search "handler" --limit 10 --fmt thin
```

---

## 15. 향후 확장(필요 시)

- 임베딩 검색: eval set의 Recall@5가 특정 쿼리 유형에서 임계값 이하일 때 도입 검토
- 더 나은 call 매칭: 모듈/임포트 힌트를 활용한 callee 해상도 개선
- CI에서 인덱스 아티팩트 생성/배포: 로컬 인덱싱 비용 절감
- LSP 연동(선택): 정확한 definition/references가 필요한 경우 외부 LSP에 위임하는 브릿지
