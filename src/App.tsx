import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, язык, поставитьЯзык, восстановитьЯзык, следить } from "./i18n";
import Agent from "./Agent";
import "./App.css";

// Реестр возможностей. Причина недоступности приходит готовой с
// Rust: если позволить интерфейсу решать, он рано или поздно
// покажет «недоступно» без причины, и это будет наша ложь.
export interface CapabilityRow {
  id: string;
  name: string;
  what: string;
  available: boolean;
  blocked_by: string | null;
  refusal: string | null;
  evidence: string | null;
  min_tier: string;
}

export interface CapabilitiesReport {
  tier: string;
  total: number;
  available: number;
  blocked: number;
  verified: number;
  rows: CapabilityRow[];
}

export interface OnboardingStep {
  step: string;
  title: string;
  index: number;
  total: number;
  what: string;
  prompt: string;
}

export interface OnboardingSay {
  understood: boolean;
  next: OnboardingStep;
  summary: string;
  finished: boolean;
}

export interface SettingsView {
  executor: { value: string; title: string; explain: string };
  approval_mode: { value: string; title: string; explain: string };
  cloud: boolean;
  deny_apps: string[];
  paths: string[];
  disabled: string[];
  warning: string | null;
}

export interface OnboardingPreview {
  summary: string;
  settings: SettingsView;
}

export interface FolderView {
  label: string;
  path: string;
}

/** Одна подсказка из поиска моделей. */
export interface ModelHit {
  repo: string;
  family: string;
  downloads: number;
}

/// Выбор из списка. Один компонент на инструменты и папки: логика
/// «отметил, подтвердил» одинакова, различаются только подписи.
function Picker({
  items,
  chosen,
  onToggle,
  onDone,
  busy,
  empty,
}: {
  items: { id: string; title: string; note: string; tag?: string | null }[];
  chosen: Set<string>;
  onToggle: (id: string, want: boolean) => void;
  onDone: () => void;
  busy: boolean;
  empty: string;
}) {
  return (
    <div className="picker">
      {items.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        items.map((r) => {
          const on = chosen.has(r.id);
          return (
            <button
              key={r.id}
              type="button"
              className={on ? "pick pick-on" : "pick"}
              onClick={() => onToggle(r.id, !on)}
              disabled={busy}
            >
              <span className="pick-box">{on ? "✓" : ""}</span>
              <span className="pick-body">
                <span className="pick-name">
                  {r.title}
                  {r.tag && <span className="cap-tag cap-tag-no">{r.tag}</span>}
                </span>
                <span className="pick-what">{r.note}</span>
              </span>
            </button>
          );
        })
      )}
      {items.length > 0 && (
        <div className="onb-input">
          <button type="button" onClick={onDone} disabled={busy}>{t("дальше")}
          </button>
        </div>
      )}
    </div>
  );
}

// Выбор инструментов кнопками. Человек не должен писать JSON, чтобы
// отметить галочку, поэтому выбор приходит списком, а не фразой.
function ToolPicker({
  rows,
  chosen,
  onToggle,
  onDone,
  busy,
}: {
  rows: CapabilityRow[];
  chosen: Set<string>;
  onToggle: (id: string, want: boolean) => void;
  onDone: () => void;
  busy: boolean;
}) {
  return (
    <>
      <p className="muted">{t("Отметь, что секретарю можно делать. Недоступное сейчас всё равно отметить можно — включишь, когда появится.")}

      </p>
      <Picker
        items={rows.map((r) => ({
          id: r.id,
          title: r.name,
          note: r.what,
          tag: r.available ? null : r.blocked_by,
        }))}
        chosen={chosen}
        onToggle={onToggle}
        onDone={onDone}
        busy={busy}
        empty={t("Возможностей в реестре нет.")}
      />
    </>
  );
}

// Профиль приходит с Rust. Пока его нет — пустой экран, без «догадок».
export interface TierNote {
  tier: string;
  describe: string;
  blocked_because: string | null;
}

export interface ProfileView {
  summary: string;
  detected: string;
  effective: string;
  forced: string | null;
  model_name: string;
  model_quant: string;
  context: number;
  kv_cache_type: string;
  gpu_name: string;
  vram_mb: number;
  vram_usable_mb: number;
  is_discrete: boolean;
  hybrid: boolean;
  ram_mb: number;
  logical_cores: number;
  cpu_brand: string;
  detection_source: string;
  unsupported_reason: string | null;
  available_tiers: TierNote[];
}

export interface ModelStatus {
  running: boolean;
  recipe: string | null;
  kv_cache: string | null;
  context: number | null;
  tok_s: number | null;
  verified: boolean;
  full_gpu: boolean;
  gpu_layers: number | null;
  total_layers: number | null;
  device: string | null;
  flash_attn: boolean;
  error: string | null;
}

interface Message {
  role: "user" | "assistant";
  content: string;
  toolCalls?: unknown;
}

function gb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} ГБ`;
}

// ---------------------------------------------------------------------------
// Экран профиля (S1)
// ---------------------------------------------------------------------------

/// Что секретарь умеет и чего не умеет, с причинами.
///
/// Экран существует не для красоты. Пока возможностей мало, список
/// читается как отчёт о недоделке — и это честнее, чем интерфейс,
/// где всё выглядит работающим, а на деле доступен разговор.
function Capabilities() {
  const [report, setReport] = useState<CapabilitiesReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<CapabilitiesReport>("capabilities_report", { lang: язык() })
      .then(setReport)
      .catch((e) => setError(String(e)));
    // Отчёт перезапрашивается при смене языка: он приходит из Rust
    // уже переведённым, и без повторного вызова переключатель оставит
    // на экране текст на прежнем языке.
  }, [язык()]);

  if (error) {
    return (
      <div className="panel">
        <h2>{t("Возможности")}</h2>
        <p className="error">{error}</p>
      </div>
    );
  }

  if (!report) {
    return (
      <div className="panel">
        <h2>{t("Возможности")}</h2>
        <p className="muted">{t("Читаем реестр…")}</p>
      </div>
    );
  }

  const ready = report.rows.filter((r) => r.available);
  const blocked = report.rows.filter((r) => !r.available);

  return (
    <div className="panel">
      <h2>{t("Возможности")}</h2>

      <div className="stat-row">
        <span className="stat"><b>{report.available}</b> {t("работает")}</span>
        <span className="stat"><b>{report.total - report.available}</b> {t("закрыто")}</span>
        <span className="stat"><b>{report.verified}</b> {t("проверено на железе")}</span>
        <span className="stat gold"><b>{report.tier}</b> {t("профиль")}</span>
      </div>

      <section className="sector">
        <h3 className="sector-head">{t("работает")}
          <span className="sector-count">{ready.length}</span>
        </h3>
        {ready.length === 0 ? (
          <p className="muted">{t("Ничего не проверено. Это значит «ещё не сделано», а не «сломалось».")}</p>
        ) : (
          <div className="cards">
            {ready.map((r) => (
              <article key={r.id} className="card ok">
                <header className="card-head">
                  <span className="card-name">{r.name}</span>
                  <span className="cap-tag cap-tag-ok">{t("работает")}</span>
                </header>
                <p className="card-what">{r.what}</p>
                {r.evidence && <p className="card-note">{t("Проверено")}: {r.evidence}</p>}
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="sector">
        <h3 className="sector-head">{t("пока недоступно")}
          <span className="sector-count dim">{blocked.length}</span>
        </h3>
        <p className="muted sector-lead">{t("Секретарь знает про эти возможности и честно скажет, что не может. Не молчит и не обещает.")}

        </p>
        <div className="cards">
          {blocked.map((r) => (
            <article key={r.id} className="card no">
              <header className="card-head">
                <span className="card-name">{r.name}</span>
                <span className="cap-tag cap-tag-no">{r.blocked_by}</span>
              </header>
              <p className="card-what">{r.what}</p>
              {r.refusal && <p className="card-refusal">{r.refusal}</p>}
            </article>
          ))}
        </div>
      </section>
    </div>
  );
}

// ---------------------------------------------------------------------------

/// Знакомство: пользователь собирает секретаря в разговоре.
///
/// Экран намеренно показывает, что будет применено, до того как
/// применяет. Разрешения нельзя отменить задним числом, и человек
/// должен видеть, что подтверждает.
function Welcome() {
  const [step, setStep] = useState<OnboardingStep | null>(null);
  const [lines, setLines] = useState<{ who: string; text: string }[]>([]);
  const [input, setInput] = useState("");
  const [preview, setPreview] = useState<OnboardingPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<CapabilitiesReport | null>(null);
  const [folders, setFolders] = useState<FolderView[]>([]);
  // Выбранные по умолчанию: то, что уже работает. Пользователь снимает
  // галочки, а не ищет, где их поставить.
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [picked, setPicked] = useState<Set<string>>(new Set());
  // Весь диск или отдельные папки. Второе — узкий вариант, и он
  // стоит по умолчанию: широкий доступ человек выдаёт сам и осознанно.
  const [wholeDisk, setWholeDisk] = useState(false);
  // Режим выполнения и уровень доступа. Пустая строка означает
  // «ещё не выбрано», и это не то же самое, что выбор не сделан:
  // пока пусто, кнопка «дальше» заблокирована.
  const [режим, setРежим] = useState("");
  const [уровень, setУровень] = useState("");

  useEffect(() => {
    // `onboarding_current`, а не `onboarding_start`: первый читает
    // текущий шаг, второй начинает с нуля.
    //
    // Разница видна не сразу, но стоит целого знакомства: компонент
    // монтируется не только при первом открытии, но и при возврате на
    // вкладку, и при перерисовке окна. Знакомство слетало на первый
    // шаг, стоило переключиться куда угодно.
    invoke<OnboardingStep>("onboarding_current", { lang: язык() })
      .then(setStep)
      .catch((e) => setError(String(e)));
    invoke<CapabilitiesReport>("capabilities_report", { lang: язык() })
      .then((r) => {
        setReport(r);
        setChosen(new Set(r.rows.filter((x) => x.available).map((x) => x.id)));
      })
      .catch(() => undefined);
    invoke<FolderView[]>("onboarding_folders")
      .then(setFolders)
      .catch(() => undefined);
  }, []);

  // Итог подгружается по самому шагу, а не в том месте, где шаг
  // закончился.
  //
  // Раньше предпросмотр брали только из свободного ответа, и путь
  // через кнопки его вовсе не касался: человек доходил до последнего
  // шага и видел пустое поле вместо «что получится». Привязка к шагу
  // закрывает все дороги сразу — и кнопки, и текст, и возврат на
  // вкладку.
  useEffect(() => {
    if (!step || step.step !== "summary" || preview) return;
    invoke<OnboardingPreview>("onboarding_preview", { lang: язык() })
      .then(setPreview)
      .catch((e) => setError(String(e)));
  }, [step, preview]);

  const pickPaths = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<OnboardingSay>("onboarding_set_paths", { lang: язык(),
        paths: Array.from(picked),
      });
      setStep(r.next);
      setLines((prev) => [...prev, { who: "вы", text: `папок выбрано: ${picked.size}` }]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Переключатель «весь диск». Отправляется сразу: на шаге папок
  // это единственный доступный выбор, и ждать отдельной кнопки
  // «дальше» было бы лишним шагом без смысла.
  const toggleWholeDisk = async (want: boolean) => {
    setBusy(true);
    setError(null);
    setWholeDisk(want);
    try {
      const r = await invoke<OnboardingSay>("onboarding_set_whole_disk", { lang: язык(),
        enabled: want,
      });
      if (want) setStep(r.next);
    } catch (e) {
      setWholeDisk(!want);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Режим и уровень доступа отправляются одной командой: по отдельности
// шаг не двигался бы, потому что решение одно — насколько секретарь
// будет действовать сам.
const отправитьРежимы = async () => {
  setBusy(true);
  setError(null);
  try {
    const r = await invoke<OnboardingSay>("onboarding_set_modes", { lang: язык(),
      executor: режим,
      approval: уровень,
    });
    setStep(r.next);
    setLines((prev) => [...prev, { who: "ты", text: "уровни выбраны" }]);
  } catch (e) {
    setError(String(e));
  } finally {
    setBusy(false);
  }
};

const pickTools = async () => {
    setBusy(true);
    setError(null);
    try {
      const enable: string[] = [];
      const disable: string[] = [];
      report?.rows.forEach((r) => {
        if (chosen.has(r.id)) enable.push(r.id);
        else disable.push(r.id);
      });
      const r = await invoke<OnboardingSay>("onboarding_set_tools", { lang: язык(), enable, disable });
      setStep(r.next);
      setLines((prev) => [
        ...prev,
        {
          who: "вы",
          text: `отмечено инструментов: ${enable.length}, снято: ${disable.length}`,
        },
      ]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const say = async () => {
    const text = input.trim();
    if (!text || !step) return;
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<OnboardingSay>("onboarding_say", { text, lang: язык() });
      setLines((prev) => [...prev, { who: "вы", text }]);
      setInput("");
      if (!r.understood) {
        // Модель ответила не тем и не разобралось. Не злимся и не
        // угадываем: тот же вопрос ещё раз.
        setError("Не понял ответ. Попробуем тот же вопрос ещё раз.");
      }
      setStep(r.next);
      if (r.finished) {
        const p = await invoke<OnboardingPreview>("onboarding_preview", { lang: язык() });
        setPreview(p);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      const p = await invoke<OnboardingPreview>("onboarding_apply", { lang: язык() });
      setPreview(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const restart = async () => {
    setLines([]);
    setPreview(null);
    setError(null);
    setStep(await invoke<OnboardingStep>("onboarding_start", { lang: язык() }));
  };

  if (error && !step) {
    return (
      <div className="panel">
        <h2>{t("Знакомство")}</h2>
        <p className="error">{error}</p>
      </div>
    );
  }

  if (!step) {
    return (
      <div className="panel">
        <h2>{t("Знакомство")}</h2>
        <p className="muted">{t("Готовлюсь…")}</p>
      </div>
    );
  }

  return (
    <div className="panel">
      <h2>{t("Знакомство")}</h2>
      <p className="muted">
        Шаг {step.index} из {step.total}: {step.title}. Собираем {step.what}.
      </p>

      <div className="onb-log">
        {lines.length === 0 ? (
          <div className="onb-bubble onb-agent">
            Сейчас {step.what}. Напиши ответ обычной фразой — я сам пойму.
          </div>
        ) : (
          lines.map((l, i) => (
            <div key={i} className={`onb-bubble ${l.who === "вы" ? "onb-user" : "onb-agent"}`}>
              {l.text}
            </div>
          ))
        )}
      </div>

      {error && <p className="error">{error}</p>}

      {step.step === "tools" && report ? (
        <ToolPicker
          rows={report.rows}
          chosen={chosen}
          busy={busy}
          onToggle={(id, want) => {
            setChosen((prev) => {
              const next = new Set(prev);
              if (want) next.add(id);
              else next.delete(id);
              return next;
            });
          }}
          onDone={pickTools}
        />
      ) : step.step === "folders" ? (
        <>
          <p className="muted">{t("Реши, насколько широко секретарь сможет читать. На этом шаге содержимое не читается — только названия папок.")}

          </p>

          {/* Сначала широкий вариант, потом узкий. Порядок не
              случайный: человек сначала решает «насколько широко»,
              и лишь потом выбирает, где именно. Обратный порядок
              заставил бы его отмечать папки, чтобы потом узнать,
              что можно было просто включить один переключатель. */}
          <button
            type="button"
            className={wholeDisk ? "wide-opt on" : "wide-opt"}
            disabled={busy}
            aria-pressed={wholeDisk}
            onClick={() => toggleWholeDisk(!wholeDisk)}
          >
            <span className="wide-title">{t("читать весь диск")}</span>
            <span className="wide-note">{t("Секретарь сможет читать что угодно в любой папке. Удобно, если документы разбросаны. Он всё равно не напишет и не удалит ничего без твоего вопроса.")}


            </span>
          </button>

          {!wholeDisk && (
            <>
              <p className="muted alt-lead">{t("или отметь отдельные папки")}</p>
              <Picker
                items={folders.map((f) => ({ id: f.label, title: f.label, note: f.path }))}
                chosen={picked}
                onToggle={(id, want) => {
                  setPicked((prev) => {
                    const next = new Set(prev);
                    if (want) next.add(id);
                    else next.delete(id);
                    return next;
                  });
                }}
                onDone={pickPaths}
                busy={busy}
                empty={t("Папок по умолчанию не нашлось. Можно назвать свою в поле ниже.")}
              />
              {folders.length === 0 && (
                <div className="onb-input">
                  <input
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") say(); }}
                    placeholder={t("путь к папке")}
                    disabled={busy}
                  />
                  <button type="button" onClick={say} disabled={busy || !input.trim()}>{t("сказать")}
                  </button>
                </div>
              )}
            </>
          )}
        </>
      ) : step.step === "levels" ? (
        <>
          <p className="muted">{t("Два решения, и оба про то, насколько секретарь будет действовать сам. Сначала — показывать или делать.")}

          </p>

          <p className="muted alt-lead">{t("режим")}</p>
          <div className="picks">
            {РЕЖИМЫ.map((m) => (
              <button
                key={m.id}
                type="button"
                className={режим === m.id ? "pick pick-on" : "pick"}
                disabled={busy}
                onClick={() => setРежим(m.id)}
              >
                <span className="pick-box">{режим === m.id ? "×" : ""}</span>
                <span className="pick-body">
                  <span className="pick-name">{m.title}</span>
                  <span className="pick-what">{m.note}</span>
                </span>
              </button>
            ))}
          </div>

          <p className="muted alt-lead">{t("уровень доступа")}</p>
          <div className="picks">
            {УРОВНИ.map((u) => (
              <button
                key={u.id}
                type="button"
                className={уровень === u.id ? "pick pick-on" : "pick"}
                disabled={busy}
                onClick={() => setУровень(u.id)}
              >
                <span className="pick-box">{уровень === u.id ? "×" : ""}</span>
                <span className="pick-body">
                  <span className="pick-name">{u.title}</span>
                  <span className="pick-what">{u.note}</span>
                </span>
              </button>
            ))}
          </div>

          <div className="onb-input">
            <button
              type="button"
              onClick={отправитьРежимы}
              disabled={busy || !режим || !уровень}
            >{t("дальше")}
            </button>
          </div>
        </>
      ) : (
        <>
          {!preview && (
            <div className="onb-input">
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") say(); }}
                placeholder={t("твой ответ")}
                disabled={busy}
              />
              <button type="button" onClick={say} disabled={busy || !input.trim()}>{t("сказать")}
              </button>
            </div>
          )}
        </>
      )}

      {preview && (
        <div className="onb-preview">
          <h3>{t("Итог")}</h3>
          <p>{preview.summary}</p>

          <div className="cap-row cap-ok">
            <div className="cap-head"><b>{t("Как буду работать")}</b></div>
            <div className="cap-what">{preview.settings.executor.title}</div>
            <div className="cap-evidence">{preview.settings.executor.explain}</div>
            <div className="cap-what" style={{ marginTop: 6 }}>
              {preview.settings.approval_mode.title}
            </div>
            <div className="cap-evidence">{preview.settings.approval_mode.explain}</div>
          </div>

          {preview.settings.paths.length > 0 && (
            <div className="cap-row cap-ok">
              <div className="cap-head"><b>{t("Папки")}</b></div>
              <div className="cap-what">{preview.settings.paths.join(", ")}</div>
            </div>
          )}

          <p className="muted">{t("Ничего ещё не применено. Проверь и нажми «сохранить», если всё верно.")}
          </p>
          <div className="onb-input">
            <button type="button" onClick={apply} disabled={busy}>{t("сохранить")}</button>
            <button type="button" onClick={restart} disabled={busy}>{t("начать заново")}</button>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------

function Settings({ profile, onChange }: {
  profile: ProfileView;
  onChange: (p: ProfileView) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function call<T>(fn: () => Promise<T>) {
    setBusy(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel">
      <h2>{t("Ваш компьютер")}</h2>
      <dl className="facts">
        <div><dt>{t("Видеокарта")}</dt><dd>{profile.gpu_name}</dd></div>
        <div>
          <dt>{t("Видеопамять")}</dt>
          <dd>
            {gb(profile.vram_mb)}
            <span className="muted"> доступно {gb(profile.vram_usable_mb)}</span>
          </dd>
        </div>
        <div><dt>{t("Процессор")}</dt><dd>{profile.cpu_brand}</dd></div>
        <div><dt>{t("Оперативная память")}</dt><dd>{gb(profile.ram_mb)}</dd></div>
      </dl>

      <h2>{t("Профиль")}</h2>
      <p className="summary">{profile.summary}</p>
      {profile.unsupported_reason && (
        <p className="error">Не поддерживается: {profile.unsupported_reason}</p>
      )}

      <ul className="tiers">
        {profile.available_tiers.map((уровень) => {
          const active = уровень.tier === profile.effective;
          const blocked = уровень.blocked_because !== null;
          return (
            // Причина и пометка лежат внутри карточки, а не рядом с
            // ней. Раньше `<em>` был соседом пункта списка и вылезал
            // за его границы — на скриншоте это и читалось как наезд
            // текста на подпись кнопки.
            <li key={уровень.tier} className={active ? "tier active" : "tier"}>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  call(() =>
                    invoke<ProfileView>("set_profile_tier", {
                      tier: active && profile.forced ? null : уровень.tier,
                    })
                  ).then((p) => p && onChange(p))
                }
                title={уровень.blocked_because ?? t("Выбрать этот профиль")}
              >
                <strong>{уровень.tier}</strong>
                <span>{уровень.describe.replace(`${уровень.tier} — `, "")}</span>
              </button>
              {blocked && <p className="tier-note muted">{уровень.blocked_because}</p>}
              {active && !blocked && <p className="tier-note chosen">{t("выбран")}</p>}
            </li>
          );
        })}
      </ul>

      {profile.forced && (
        <button type="button" className="link" disabled={busy}
          onClick={() => call(() => invoke<ProfileView>("set_profile_tier", { tier: null }))
            .then((p) => p && onChange(p))}>{t("вернуть автоматическое определение")}
        </button>
      )}

      <h2>{t("Контекст")}</h2>
      <p className="muted">{t("Сколько токенов модель держит в голове за раз. Больше — больше помнит, но дольше думает: на 24 000 токенах первый ответ приходит через 11 секунд.")}


      </p>
      <div className="context-row">
        {[16384, 32768, 65536, 131072, 262144].map((c) => (
          <button key={c} type="button" disabled={busy}
            className={c === profile.context ? "chip active" : "chip"}
            onClick={() => call(() => invoke<ProfileView>("set_context", { context: c }))
              .then((p) => p && onChange(p))}>
            {c / 1024}K
          </button>
        ))}
      </div>

      {error && <p className="error">{error}</p>}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Чат (S2)
// ---------------------------------------------------------------------------

function Chat({ model }: { model: ModelStatus }) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const requestId = useRef(0);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [
      listen<{ request_id: number; delta: string }>("chat:chunk", (e) => {
        if (e.payload.request_id !== requestId.current) return;
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") {
            next[next.length - 1] = { ...last, content: last.content + e.payload.delta };
          }
          return next;
        });
      }),
      listen<{
        request_id: number; outcome: string; prompt_tokens: number | null;
        completion_tokens: number | null; tok_s: number | null;
        error: string | null; tool_calls: unknown;
      }>("chat:done", (e) => {
        if (e.payload.request_id !== requestId.current) return;
        setBusy(false);
        const p = e.payload;
        if (p.error) {
          setNotice(p.error);
        } else {
          const bits: string[] = [];
          if (p.prompt_tokens) bits.push(`${p.prompt_tokens} в промпте`);
          if (p.tok_s) bits.push(`${p.tok_s.toFixed(1)} tok/s`);
          if (p.outcome === "stop") bits.push("остановлено");
          setNotice(bits.join(" · ") || null);
        }
      }),
    ];
    return () => { unlisteners.forEach((p) => p.then((f) => f())); };
  }, []);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || busy || !model.running) return;

    const next: Message[] = [...messages, { role: "user", content: text }];
    setMessages([...next, { role: "assistant", content: "" }]);
    setInput("");
    setBusy(true);
    setNotice(null);

    try {
      requestId.current = await invoke<number>("chat", {
        messages: next.map(({ role, content }) => ({ role, content })),
        maxTokens: 1024,
      });
    } catch (e) {
      setBusy(false);
      setNotice(String(e));
      setMessages(next);
    }
  }, [busy, input, messages, model.running]);

  return (
    <section className="chat">
      <div className="messages">
        {messages.length === 0 && (
          <p className="muted">
            Модель {model.recipe ?? "не выбрана"} готова. Напишите что-нибудь.
          </p>
        )}
        {messages.map((m, i) => (
          <div key={i} className={`msg ${m.role}`}>{m.content || "…"}</div>
        ))}
        <div ref={bottom} />
      </div>

      {busy && (
        <button type="button" className="stop" onClick={() => invoke("stop_generation")}>{t("остановить ответ")}
        </button>
      )}
      {notice && <p className="muted notice">{notice}</p>}

      <form className="composer" onSubmit={(e) => { e.preventDefault(); send(); }}>
        <input
          className="app-input"
          value={input}
          onChange={(e) => setInput(e.currentTarget.value)}
          placeholder={model.running ? "Напишите сообщение" : "Сначала запустите модель"}
          disabled={!model.running || busy}
          aria-label={t("Сообщение")}
        />
        <button className="app-send" type="submit" disabled={!input.trim() || busy || !model.running}>{t("Отправить")}
        </button>
      </form>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Модель
// ---------------------------------------------------------------------------

interface Problem {
  what: string;
  why: string;
  how: string;
}

interface Preflight {
  problems: Problem[];
  usable_recipe: string | null;
  models_dir: string;
}

interface RemoteFile {
  path: string;
  size: number;
  sha256: string;
}

type DownloadState =
  | { kind: "idle" }
  | { kind: "meta" }
  | { kind: "busy"; downloaded: number; total: number; mib: number }
  | { kind: "verifying"; size: number }
  | { kind: "done"; path: string }
  | { kind: "error"; message: string };

function gib(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(2)} ГиБ`;
}

/// Что приложение знает о движке. Имена полей латиницей — их
/// отдаёт Rust, и переводить кириллицу на стороне TypeScript
/// незачем.
interface EngineState {
  found: boolean;
  path: string | null;
  searched: string[];
  cudaNeeded: boolean;
  cudaPresent: boolean;
  gpuName: string;
}

type EngineProgress =
  | { state: "ищем"; assets: string[] }
  | {
      state: "качаем";
      файл: string;
      downloaded: number;
      total: number;
      megabytes_per_s: number;
    }
  | { state: "распаковка"; файл: string; куда: string }
  | { state: "готово"; в_какую_папку: string };

/// Панель движка: что нашлось и чего не хватает.
///
/// Показывается всегда, а не только когда что-то сломалось.
/// Человек должен видеть, что программа работает с процессором и
/// картой не пользуется, пока он сам этого не попросил.
function EnginePanel() {
  const [state, setState] = useState<EngineState | null>(null);
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  const [ошибка, setОшибка] = useState<string | null>(null);

  const опросить = () =>
    invoke<EngineState>("engine_status").then(setState).catch(() => undefined);

  useEffect(() => {
    опросить();
  }, []);

  useEffect(() => {
    const un = listen<EngineProgress>("engine://progress", (e) => {
      setProgress(e.payload);
      if (e.payload.state === "готово") опросить();
    });
    return () => { un.then((f) => f()); };
  }, []);

  const качать = async () => {
    setОшибка(null);
    setProgress({ state: "ищем", assets: [] });
    try {
      const итог = await invoke<EngineState>("engine_download_cuda");
      setState(итог);
      setProgress(null);
    } catch (e) {
      setProgress(null);
      setОшибка(String(e));
    }
  };

  if (!state) return null;

  const идёт = progress !== null;

  return (
    <section className="panel">
      <h2>{t("Движок")}</h2>

      <p className="muted">
        {state.found
          ? t("Движок найден. Работает на процессоре.")
          : t("Движок не найден ни рядом с программой, ни в папке проекта. Модель не запустится.")}
      </p>

      {state.gpuName && (
        <p className="muted">{t("Видеокарта: ")}{state.gpuName}</p>
      )}

      {state.cudaPresent ? (
        <p className="muted">{t("Библиотеки карты на месте — движок работает на видеокарте.")}</p>
      ) : state.cudaNeeded ? (
        <>
          <p>
            {t("Видеокарта есть, а библиотек для неё нет: ")}
            <strong>523 МиБ</strong>
            {t(". Без них агент пойдёт на процессоре, медленно. Скачивание делаете вы, и только один раз.")}
          </p>
          <button onClick={качать} disabled={идёт}>
            {идёт ? t("Идёт загрузка…") : t("Скачать библиотеки карты")}
          </button>
        </>
      ) : (
        <p className="muted">
          {t("Библиотеки карты не нужны: видеокарты NVIDIA на этой машине нет. Модель работает на процессоре.")}
        </p>
      )}

      {progress && progress.state === "качаем" && (
        <p className="muted">
          {t("Качаем: ")}
          {(progress.downloaded / 1048576).toFixed(0)}
          {" / "}
          {(progress.total / 1048576).toFixed(0)}
          {" МиБ, "}
          {progress.megabytes_per_s.toFixed(1)}
          {" МиБ/с"}
        </p>
      )}
      {progress && progress.state === "распаковка" && (
        <p className="muted">{t("Распаковываем: ")}{progress.файл}</p>
      )}

      {ошибка && <p className="muted">{ошибка}</p>}

      {!state.found && (
        <p className="muted">
          {t("Искали здесь:")}
          <br />
          {state.searched.map((p) => (
            <span key={p}>{p}</span>
          ))}
        </p>
      )}
    </section>
  );
}

function Downloader({ model }: { model: ModelStatus }) {
  const [repo, setRepo] = useState("unsloth/Qwen3.5-9B-GGUF");
  const [files, setFiles] = useState<RemoteFile[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [state, setState] = useState<DownloadState>({ kind: "idle" });
  const [hosts, setHosts] = useState<string[] | null>(null);
  // Подсказки по семейству. null означает «ещё не искали», пустой
  // массив — «искали, ничего не нашлось». Различать надо: при null
  // список скрывается, при пустом показывается подпись.
  const [hits, setHits] = useState<ModelHit[] | null>(null);
  const [ищет, setИщет] = useState(false);

  useEffect(() => {
    invoke<string[]>("download_hosts")
      .then(setHosts)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    const un = listen<DownloadState>("download:progress", (e) => {
      setState(e.payload as DownloadState);
    });
    return () => { un.then((f) => f()); };
  }, []);

  // Поиск с паузой. Без паузы запрос уходит на каждый символ:
  // это заметно и по сети, и по тому, что список дёргается.
  useEffect(() => {
    const запрос = repo.trim();
    if (запрос.length < 2) {
      setHits(null);
      return;
    }
    setИщет(true);
    const таймер = setTimeout(() => {
      invoke<ModelHit[]>("search_models", { query: запрос })
        .then(setHits)
        .catch(() => setHits([]))
        .finally(() => setИщет(false));
    }, 350);
    // Гонка: человек печатает быстро, и ответ на «gem» может
    // прийти после ответа на «gemma». Ответ на устаревший запрос
    // отбрасывается, иначе список дёргался бы назад.
    return () => clearTimeout(таймер);
  }, [repo]);

  async function loadFiles() {
    setState({ kind: "meta" });
    try {
      const list = await invoke<RemoteFile[]>("repo_files", { repo });
      setFiles(list.filter((f) => f.path.endsWith(".gguf")));
      setState({ kind: "idle" });
    } catch (e) {
      setFiles(null);
      setState({ kind: "error", message: String(e) });
    }
  }

  async function start() {
    if (!picked) return;
    setState({ kind: "meta" });
    try {
      await invoke<string>("download_model", { repo, file: picked });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  return (
    <section className="panel">
      <h2>{t("Скачать модель")}</h2>

      {model.running ? (
        <p className="muted">
          Сейчас работает «{model.recipe}». Скачивание новой модели
          ничего не сломает, но запускать её придётся заново.
        </p>
      ) : (
        <p className="muted">{t("Под текущий профиль нужен квант из списка. Показан размер файла, а не скорость: она станет известна после сверки сводки.")}

        </p>
      )}

      <div className="repo-row">
        <input
          className="app-input"
          value={repo}
          onChange={(e) => setRepo(e.currentTarget.value)}
          placeholder={t("начни набирать: gemma, qwen, llama…")}
          aria-label={t("Репозиторий")}
        />
        <button className="chip" onClick={loadFiles}
          disabled={state.kind === "meta" || !repo.trim()}>{t("показать файлы")}
        </button>
      </div>

      {/* Подсказки по семейству. Список живёт под полем и исчезает
          сразу после выбора: оставшаяся строка подсказки закрывала бы
          то, ради чего человек в неё ткнул. */}
      {hits && hits.length > 0 && (
        <ul className="hints">
          {hits.map((h) => (
            <li key={h.repo}>
              <button
                type="button"
                onClick={() => { setRepo(h.repo); setHits(null); }}
              >
                <span className="hint-family">{h.family}</span>
                <span className="hint-repo">{h.repo}</span>
                <span className="hint-dl">
                  {h.downloads > 999 ? `${(h.downloads / 1000).toFixed(0)}k` : h.downloads}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {ищет && <p className="muted hint-status">{t("ищем…")}</p>}
      {hits && hits.length === 0 && !ищет && (
        <p className="muted hint-status">{t("по этому слову моделей не нашлось. Проверь написание или укажи репозиторий целиком.")}

        </p>
      )}

      {files && files.length > 0 && (
        <ul className="files">
          {files.map((f) => (
            <li key={f.path}>
              <button
                type="button"
                className={picked === f.path ? "file active" : "file"}
                onClick={() => setPicked(f.path)}
              >
                <span className="file-name">
                  {f.path.split("/").pop()}
                </span>
                <span className="muted">{gib(f.size)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {state.kind === "busy" && (
        <div className="progress">
          <div className="progress-bar">
            <div
              className="progress-fill"
              style={{ width: `${(state.downloaded / state.total) * 100}%` }}
            />
          </div>
          <p className="muted">
            {gib(state.downloaded)} из {gib(state.total)}, {state.mib.toFixed(1)} МБ/с
          </p>
          <button className="stop" onClick={() => invoke("cancel_download")}>{t("отменить")}
          </button>
        </div>
      )}

      {state.kind === "verifying" && (
        <p className="muted">Проверяем сводку файла, {gib(state.size)}…</p>
      )}

      {state.kind === "done" && (
        <p className="ok">Готово: {state.path}</p>
      )}

      {state.kind === "error" && (
        <p className="error">{state.message}</p>
      )}

      {picked && (
        <button className="app-send" onClick={start}
          disabled={state.kind === "busy" || state.kind === "meta"}>{t("Скачать")}
        </button>
      )}

      {hosts && (
        <details className="hosts">
          <summary className="muted">{t("если не скачивается: нужны разрешённые домены")}
          </summary>
          <ul>
            {hosts.map((h) => <li key={h}>{h}</li>)}
          </ul>
        </details>
      )}
    </section>
  );
}

function Problems({ pre }: { pre: Preflight }) {
  return (
    <section className="panel">
      <h2>{t("Что мешает запуску")}</h2>
      <ol className="problems">
        {pre.problems.map((p, i) => (
          <li key={i}>
            <strong>{p.what}</strong>
            <p className="muted">{p.why}</p>
            <p className="how">{p.how}</p>
          </li>
        ))}
      </ol>
      <p className="muted small">папка моделей: {pre.models_dir}</p>
    </section>
  );
}

function ModelBar({ model, onChange }: { model: ModelStatus; onChange: (m: ModelStatus) => void }) {
  const [starting, setStarting] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [showLog, setShowLog] = useState(false);
  const [pre, setPre] = useState<Preflight | null>(null);

  async function start() {
    setStarting(true);
    try {
      onChange(await invoke<ModelStatus>("start_model"));
    } catch (e) {
      onChange({ ...model, running: false, error: String(e) });
      setPre(await invoke<Preflight>("preflight_model").catch(() => null));
    } finally {
      setStarting(false);
    }
  }

  async function check() {
    setPre(await invoke<Preflight>("preflight_model").catch(() => null));
  }

  useEffect(() => {
    if (model.error) check();
  }, [model.error]);

  async function toggleLog() {
    const next = !showLog;
    setShowLog(next);
    if (next) {
      setLog(await invoke<string[]>("model_log", { lines: 60 }));
    }
  }

  return (
    <section className="panel">
      <h2>{t("Модель")}</h2>

      {!model.running ? (
        <button type="button" className="app-send" disabled={starting} onClick={start}>
          {starting ? "Проверяем и запускаем…" : "Запустить модель"}
        </button>
      ) : (
        <>
          <dl className="facts">
            <div><dt>{t("Рецепт")}</dt><dd>{model.recipe}</dd></div>
            <div><dt>{t("KV-кэш")}</dt><dd>{model.kv_cache}</dd></div>
            <div><dt>{t("Контекст")}</dt><dd>{model.context ? `${model.context / 1024}K` : "—"}</dd></div>
            <div>
              <dt>{t("Размещение")}</dt>
              <dd className={model.full_gpu ? "ok" : "bad"}>
                {model.full_gpu
                  ? `все слои на GPU (${model.gpu_layers}/${model.total_layers})`
                  : `частично: ${model.gpu_layers}/${model.total_layers}`}
              </dd>
            </div>
            <div><dt>{t("Устройство")}</dt><dd>{model.device ?? "—"}</dd></div>
            <div>
              <dt>{t("Скорость")}</dt>
              <dd>
                {model.tok_s ? `${model.tok_s} tok/s` : "не измерена"}
                {!model.verified && <span className="muted">{t("· рецепт не замерян")}</span>}
              </dd>
            </div>
          </dl>

          <div className="context-row">
            <button type="button" className="chip" onClick={() => invoke("stop_model")}>{t("остановить модель")}
            </button>
            <button type="button" className="chip" onClick={toggleLog}>
              {showLog ? "скрыть журнал" : "журнал запуска"}
            </button>
          </div>
        </>
      )}

      {model.error && <p className="error">{model.error}</p>}
      {pre && <Problems pre={pre} />}

      {showLog && (
        <pre className="log">{log.join("\n") || "журнал пуст"}</pre>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Приложение
// ---------------------------------------------------------------------------

type Tab = "chat" | "agent" | "model" | "download" | "capabilities" | "welcome" | "settings" | "prefs";

/**
 * Кегль интерфейса.
 *
 * Размер шрифта хранится в самом интерфейсе, а не в настройках
 * секретаря: это свойство окна, а не программы, и человек меняет
 * его, когда глаза устали, а не когда перенастраивает доступ.
 * Значение кладётся в CSS-переменную на корне, поэтому меняется
 * ровно то, что нарисовано, без перезагрузки и без перезапуска.
 */
const РАЗМЕРЫ_ШРИФТА = [13, 14, 15, 16, 18] as const;

/**
 * Варианты для шага «Уровни доступа».
 *
 * Идентификаторы совпадают с тем, что ждёт Rust: `dry-run`/`live`
 * и `actions`/`all`/`never`. Название и пояснение живут здесь, а не
 * приходят с сервера: на этом шаге человек принимает решение о том,
 * насколько секретарь будет действовать сам, и читать его должно
 * прямо на кнопке, а не после запроса.
 */
const РЕЖИМЫ = [
  {
    id: "dry-run",
    title: "только показывать",
    note: "Секретарь пишет, что собирается сделать, и ждёт. Ничего не происходит без твоего слова.",
  },
  {
    id: "live",
    title: "выполнять",
    note: "Согласные действия выполняются сразу. Опасные — всё равно спросит.",
  },
];

const УРОВНИ = [
  {
    id: "actions",
    title: "опасные — спросить",
    note: "Удаление, запуск программ и переписывание файлов требуют твоего подтверждения.",
  },
  {
    id: "all",
    title: "спросить всё",
    note: "Подтверждения на каждое действие. Медленнее, зато предсказуемо.",
  },
  {
    id: "never",
    title: "не спрашивать",
    note: "Делает всё, что разрешено. Опасно: промах в команде он не остановит.",
  },
];

function РазмерШрифта() {
  const [кегль, setКегль] = useState<number>(() => {
    const сохранён = Number(localStorage.getItem("arkha-font-size"));
    return РАЗМЕРЫ_ШРИФТА.includes(сохранён as (typeof РАЗМЕРЫ_ШРИФТА)[number])
      ? сохранён
      : 14;
  });

  useEffect(() => {
    document.documentElement.style.setProperty("--кегль", `${кегль}px`);
    localStorage.setItem("arkha-font-size", String(кегль));
  }, [кегль]);

  const шаг = РАЗМЕРЫ_ШРИФТА.indexOf(кегль as (typeof РАЗМЕРЫ_ШРИФТА)[number]);

  return (
    <section className="panel">
      <h2>{t("настройки")}</h2>

      <div className="pref-row">
        <div className="pref-text">
          <b>{t("Размер шрифта")}</b>
          <span className="muted">{t("Меняет весь интерфейс сразу. Если текст мелкий — поставь побольше, секретарь от этого не тупеет.")}

          </span>
        </div>
        <div className="stepper">
          <button
            type="button"
            aria-label={t("мельче")}
            disabled={шаг <= 0}
            onClick={() => setКегль(РАЗМЕРЫ_ШРИФТА[шаг - 1] ?? кегль)}
          >
            A−
          </button>
          <span className="stepper-val">{кегль}</span>
          <button
            type="button"
            aria-label={t("крупнее")}
            disabled={шаг >= РАЗМЕРЫ_ШРИФТА.length - 1}
            onClick={() => setКегль(РАЗМЕРЫ_ШРИФТА[шаг + 1] ?? кегль)}
          >
            A+
          </button>
        </div>
      </div>

      <p className="muted pref-sample" style={{ fontSize: `${кегль}px` }}>{t("Образец: Проверка распознавания речи. Сегодня вторник.")}
      </p>
    </section>
  );
}

/**
 * Цвет смысла раздела.
 *
 * Золото — действие, которое запускает работу: модель, загрузка,
 * знакомство. Фиолетовый — сам секретарь, то, что думает и
 * действует само. Синий — передача данных: чат и профиль.
 *
 * Цвет здесь код, а не украшение. Подпись читается медленно и
 * требует фокуса внимания, а точка под вкладкой видна боковым
 * зрением, и человек узнаёт раздел раньше, чем прочитает его.
 */
const ТОН: Record<Tab, "gold" | "blue"> = {
  model: "gold",
  agent: "gold",
  chat: "blue",
  download: "gold",
  capabilities: "blue",
  welcome: "gold",
  settings: "blue",
  prefs: "blue",
};

/** Состояние голоса, как его отдаёт `voice_status`. */
interface VoiceState {
  on: boolean;
  listening: boolean;
  weightMb: number | null;
}

/**
 * Переключатель голоса.
 *
 * Голос включается только по нажатию человека, и это не удобство, а
 * необходимость: поднятое распознавание занимает место, которое на
 * 8 ГиБ нужно агенту. Выключенный голос не занимает ничего — поэтому
 * он и выключен по умолчанию, а человек включает его на время разговора.
 */
function VoiceToggle() {
  const [state, setState] = useState<VoiceState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<VoiceState>("voice_status").then(setState).catch(() => undefined);
  }, []);

  const переключить = async () => {
    setBusy(true);
    setError(null);
    try {
      const следующее = state?.on
        ? await invoke<VoiceState>("voice_disable")
        : await invoke<VoiceState>("voice_enable");
      setState(следующее);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!state) return <span className="voice">{t("голос: …")}</span>;

  return (
    <span className="voice">
      <button
        type="button"
        className={state.on ? "chip on" : "chip"}
        disabled={busy}
        onClick={переключить}
        title={state.on
          ? t("выключить голос и освободить место")
          : t("включить голос: поднимется распознавание")}
      >
        {busy ? "…" : state.on ? t("голос включён") : t("включить голос")}
      </button>
      {state.on && state.weightMb !== null && (
        <span className="voice-mb">{state.weightMb} МБ</span>
      )}
      {error && <span className="voice-error" title={error}>{t("не вышло")}</span>}
    </span>
  );
}

/** Снимок памяти, как его отдаёт команда `память_сейчас`. */
interface MemoryView {
  ramUsedMb: number;
  ramTotalMb: number;
  vramUsedMb: number | null;
  vramTotalMb: number | null;
  vramНеизвестна: boolean;
}

/** Гигабайты для подписи, одна значащая цифра после запятой. */
function гиг(миб: number): string {
  return (миб / 1024).toFixed(1);
}

/**
 * Полоса памяти в подвале.
 *
 * Показывает оперативную всегда, видеопамять — только если счётчик
 * ответил. На машине без NVIDIA видеопамяти в этом смысле нет, и
 * пустая полоса вводила бы в заблуждение: выглядело бы как «занято
 * 0 из 0», то есть как поломка.
 *
 * Опрос раз в две секунды. Чаще — заметно на глаз, когда полоса
 * дёргается; реже — человек не видит, что место кончилось, вплоть до
 * отказа модели.
 */
function MemoryBar() {
  const [mem, setMem] = useState<MemoryView | null>(null);

  useEffect(() => {
    let жив = true;
    const опрос = async () => {
      try {
        const снимок = await invoke<MemoryView>("память_сейчас");
        if (жив) setMem(снимок);
      } catch {
        // Снимок не пришёл: полоса просто остаётся как была.
        // Ошибку здесь не показываем — она повторилась бы каждые
        // две секунды и забила собой подвал.
      }
    };
    опрос();
    const таймер = setInterval(опрос, 2000);
    return () => {
      жив = false;
      clearInterval(таймер);
    };
  }, []);

  if (!mem || mem.ramTotalMb === 0) {
    return <span className="mem">{t("память: …")}</span>;
  }

  const ram = Math.min(100, Math.round((mem.ramUsedMb * 100) / mem.ramTotalMb));
  const vram = mem.vramНеизвестна || !mem.vramTotalMb
    ? null
    : Math.min(100, Math.round(((mem.vramUsedMb ?? 0) * 100) / mem.vramTotalMb));

  return (
    <span className="mem">
      <span className="mem-item" title={`оперативная память: ${гиг(mem.ramUsedMb)} из ${гиг(mem.ramTotalMb)} ГиБ`}>
        <span className="mem-label">{t("ОЗУ")}</span>
        <span className="bar"><span className="fill" style={{ width: `${ram}%` }} /></span>
        <span className="mem-num">{ram}%</span>
      </span>
      {vram !== null && (
        <span className="mem-item" title={`видеопамять: ${гиг(mem.vramUsedMb ?? 0)} из ${гиг(mem.vramTotalMb ?? 0)} ГиБ`}>
          <span className="mem-label">VRAM</span>
          <span className="bar"><span className="fill" style={{ width: `${vram}%` }} /></span>
          <span className="mem-num">{vram}%</span>
        </span>
      )}
    </span>
  );
}

/**
 * Переключатель языка.
 *
 * Кнопка, а не список: двух языка ровно два, и список ради двух
 * пунктов превращает переключатель в лишний шаг.
 *
 * Подпись на кнопке всегда «на том языке, который получишь»:
 * RU надписью «EN» ждёт нажатия, RU надписью «RU» не ждёт ничего и
 * читается как «уже русский». Так человек не гадает, что нажатие
 * сделает.
 */
function ЯзыкКнопка() {
  const текущийЯзык = язык();
  return (
    <button
      type="button"
      className="lang"
      title={текущийЯзык === "ru" ? "Switch to English" : "Переключить на русский"}
      aria-label={текущийЯзык === "ru" ? "Switch to English" : "Переключить на русский"}
      onClick={() => поставитьЯзык(текущийЯзык === "ru" ? "en" : "ru")}
    >
      {текущийЯзык === "ru" ? "EN" : "RU"}
    </button>
  );
}

function App() {
  const [profile, setProfile] = useState<ProfileView | null>(null);
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("model");
  // Пустое состояние, которое меняется при смене языка: `t()` читает
  // модульную переменную, и React об этом не знает. Подписка шлёт
  // событие, подписчик меняет состояние, дерево перерисовывается.
  const [, перерисовать] = useState(0);

  useEffect(() => {
    восстановитьЯзык();
    return следить(() => перерисовать((n) => n + 1));
  }, []);

  useEffect(() => {
    invoke<ProfileView>("get_profile").then(setProfile).catch((e) => setError(String(e)));
    invoke<ModelStatus>("model_status").then(setModel).catch(() => undefined);

    // Модель может упасть сама: не хватило видеопамяти, сбойнул
    // драйвер. Без опроса интерфейс вечно показывал бы «запущена» и
    // кнопку «остановить», которая не останавливает ничего, потому
    // что останавливать уже нечего. Пять секунд — достаточно часто,
    // чтобы падение заметил человек, и достаточно редко, чтобы не
    // дёргать окно.
    const опрос = setInterval(() => {
      invoke<ModelStatus>("model_status")
        .then(setModel)
        .catch(() => undefined);
    }, 5000);
    return () => clearInterval(опрос);
  }, []);

  if (!model) {
    return (
      <main className="app">
        <header className="app-header">
          <div className="brand">
            <span className="brand-mark">ARKHA</span>
            <span className="app-subtitle">{t("локальный секретарь")}</span>
          </div>
        </header>
        <div className="res-strip">
          <span className="res"><span className="res-label">{t("статус")}</span><span className="res-val">{t("загрузка…")}</span></span>
        </div>
        <section className="app-body"><p className="app-hint">{t("Поднимаем интерфейс…")}</p></section>
      </main>
    );
  }

  return (
    <main className="app">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark">ARKHA</span>
          <span className="app-subtitle">{t("локальный секретарь")}</span>
        </div>
        <nav className="tabs">
          {(["model", "agent", "chat", "download", "capabilities", "welcome", "settings", "prefs"] as Tab[]).map((вкладка) => (
            <button key={вкладка} type="button"
              data-tone={ТОН[вкладка]}
              className={tab === вкладка ? "tab active" : "tab"}
              onClick={() => setTab(вкладка)}>
              {t({
                model: "модель",
                agent: "секретарь",
                chat: "чат",
                download: "загрузка",
                capabilities: "возможности",
                welcome: "Знакомство",
                settings: "профиль",
                prefs: "настройки",
              }[вкладка])}
            </button>
          ))}
        </nav>
        <ЯзыкКнопка />
      </header>

      <div className="res-strip">
        <span className="res">
          <span className="res-label">{t("профиль")}</span>
          <span className="res-val gold">{profile ? profile.effective : "…"}</span>
        </span>
        <span className="res">
          <span className="res-label">{t("модель")}</span>
          <span className={`res-val ${model.running ? "ok" : "muted"}`}>
            {model.running ? (model.recipe ?? t("запущена")) : t("не запущена")}
          </span>
        </span>
        {model.running && model.tok_s !== null && model.tok_s !== undefined && (
          <span className="res">
            <span className="res-label">{t("скорость")}</span>
            <span className="res-val">{model.tok_s.toFixed(1)} ток/с</span>
          </span>
        )}
        <MemoryBar />
        <VoiceToggle />
      </div>

      <section className="app-body">
        {error && <p className="error">Не удалось определить профиль: {error}</p>}
        {tab === "model" && <ModelBar model={model} onChange={setModel} />}
        {tab === "agent" && <Agent />}
        {tab === "chat" && <Chat model={model} />}
        {tab === "download" && (
          <>
            <EnginePanel />
            <Downloader model={model} />
          </>
        )}
        {tab === "capabilities" && <Capabilities />}
        {tab === "welcome" && <Welcome />}
        {tab === "settings" && profile && <Settings profile={profile} onChange={setProfile} />}
        {tab === "prefs" && <РазмерШрифта />}
      </section>

      <footer className="app-footer">
        <span className="hint"><kbd className="cap">1</kbd>–<kbd className="cap">8</kbd> вкладки</span>
        <span className="hint"><kbd className="cap">Enter</kbd> отправить</span>
        <span className="hint"><kbd className="cap">Esc</kbd> остановить</span>
        <span className="hint dim">{t("всё считается на этой машине, облако выключено")}</span>
      </footer>
    </main>
  );
}

export default App;