// Вкладка агента.
//
// Главное отличие от вкладки чата: пользователь видит не только ответ,
// но и **что секретарь делал**. Инструмент, который сработал, показан
// строкой; отказ — строкой с причиной.
//
// Почему это не украшение. Агент читает файлы с вашего диска. Если
// он молча прочитал не тот файл или не смог прочитать нужный, вы
// должны это увидеть сразу, а не узнать из ответа, который агент
// составил по неполным данным.
//
// Причина остановки показывается всегда, даже при успехе. «Ответ
// готов» и «остановлено на 12 шагах, агент зациклился» выглядят
// снаружи одинаково — пустой ответом, — а разница между ними вся.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface CallRecord {
  tool: string;
  ok: boolean;
  summary: string;
  denied: boolean;
}

interface DoneEvent {
  request_id: number;
  answer: string;
  calls: CallRecord[];
  turns: number;
  stop_reason: string;
  error: string | null;
  basis?: { read: string[]; ok: number; denied: number; label: string };
  safety_fired?: boolean;
  compacted?: string;
}

interface PendingEvent {
  request_id: number;
  pending: {
    call_id: string;
    tool: string;
    capability: string;
    arguments: Record<string, unknown>;
    reason: string;
    level: string;
  };
}

/// Решение пользователя. Значения совпадают с `approvals::Решение`
/// в Rust: `kebab-case` приходит на провод без изменений.
type Решение = "один-раз" | "всю-задачу" | "отказ";

interface ToolsReport {
  tools: string[];
  capabilities: string[];
  can_change: boolean;
  навыки?: string[];
  сломанные_навыки?: string[];
  исполнитель?: string;
  подтверждения?: string;
}

interface Реплика {
  роль: "я" | "секретарь" | "инструмент" | "отказ";
  текст: string;
  /// Название инструмента для строк о вызовах. Отдельное поле, а не
  /// часть текста: склеивать имя и суть через разделитель значит
  /// потом разбирать строку обратно, и любая опечатка в сути ломает
  /// разбор.
  инструмент?: string;
  /// На чём основан ответ секретаря.
  ///
  /// Показывается рядом с текстом и не прячется. Если прочитано
  /// было что-то — видно что. Если ничего — видно и это, потому что
  /// правдоподобный выдуманный ответ иначе читается как знание.
  основание?: string;
}

// Как называем инструмент по-человечески. Ключ — идентификатор из
// реестра. Неизвестный инструмент показываем как есть: скрывать его
// нельзя, иначе в интерфейсе появится вызов, которого не видно.
const НАЗВАНИЯ: Record<string, string> = {
  "fs.read": "чтение файла",
  "fs.list": "просмотр папки",
  "skill.run": "навык",
  "windows.list": "список окон",
  "windows.find": "поиск окна",
  "windows.focus": "переключение окна",
  "windows.type": "ввод текста",
  "windows.key": "нажатие клавиш",
  "windows.click": "нажатие кнопки",
  "apps.launch": "запуск программы",
};

function название(инструмент: string): string {
  return НАЗВАНИЯ[инструмент] ?? инструмент;
}

export default function Agent() {
  const [вопрос, setВопрос] = useState("");
  const [реплики, setРеплики] = useState<Реплика[]>([]);
  const [крутится, setКрутится] = useState(false);
  const [причина, setПричина] = useState<string | null>(null);
  const [инструменты, setИнструменты] = useState<ToolsReport | null>(null);
  const [ждёт, setЖдёт] = useState<(PendingEvent & { отвечено?: boolean }) | null>(null);
  const [сжато, setСжато] = useState<string | null>(null);

  // История для следующего запроса. Отдельная от видимых реплик: в
  // модель уходят только роли и текст, без наших пометок.
  const история = useRef<{ role: string; content: string }[]>([]);
  const низ = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    invoke<ToolsReport>("agent_tools")
      .then(setИнструменты)
      .catch((e) => setПричина(String(e)));
  }, []);

  useEffect(() => {
    низ.current?.scrollIntoView({ behavior: "smooth" });
  }, [реплики, крутится]);

  useEffect(() => {
    const unlisten: Array<() => void> = [];

    // Событие приходит после того, как интерфейс уже нарисован.
    // Подписка должна быть готова раньше запроса, иначе первый ответ
    // пропадёт: команда возвращает идентификатор и сразу работает.
    listen<DoneEvent>("agent:done", (e) => {
      const d = e.payload;
      setКрутится(false);
      setЖдёт(null);
      setПричина(d.stop_reason === "ответ готов" ? null : d.stop_reason);
      setСжато(d.compacted && d.compacted.length > 0 ? d.compacted : null);
      if (d.answer.trim()) {
        setРеплики((старые) => [
          ...старые,
          {
            роль: "секретарь",
            текст: d.answer,
            основание: d.basis?.label,
          },
        ]);
        история.current.push({ role: "assistant", content: d.answer });
      }
    }).then((f) => unlisten.push(f));

    listen<PendingEvent>("agent:pending", (e) => {
      setЖдёт(e.payload);
      setКрутится(false);
    }).then((f) => unlisten.push(f));

    // Вызовы показываются по мере выполнения, а не в конце. Иначе
    // пользователь смотрит на «работает…» и не понимает, читает
    // секретарь ему файлы или завис.
    listen<{ request_id: number; call: CallRecord }>("agent:call", (e) => {
      const { call } = e.payload;
      setРеплики((старые) => [
        ...старые,
        {
          роль: call.denied ? "отказ" : "инструмент",
          инструмент: call.tool,
          текст: call.summary,
        },
      ]);
    }).then((f) => unlisten.push(f));

    return () => unlisten.forEach((f) => f());
  }, []);

  async function сменитьРежим(действие: () => Promise<ToolsReport>) {
    try {
      setИнструменты(await действие());
    } catch (e) {
      setПричина(String(e));
    }
  }

  async function решить(решение: Решение) {
    // Кнопка гасится сразу: повторное нажатие отправило бы второе
    // решение в очередь, и следующий вопрос получил бы чужой ответ.
    setЖдёт(null);
    try {
      await invoke("agent_decide", { decision: решение });
    } catch (e) {
      setПричина(String(e));
    }
  }

  async function спросить() {
    const текст = вопрос.trim();
    if (!текст || крутится) return;

    setВопрос("");
    setРеплики((старые) => [...старые, { роль: "я", текст }]);
    setПричина(null);
    setКрутится(true);

    const сообщения = [...история.current, { role: "user", content: текст }];
    история.current = сообщения;

    try {
      await invoke<number>("agent_chat", { messages: сообщения });
    } catch (e) {
      setКрутится(false);
      setПричина(String(e));
    }
  }

  function остановить() {
    invoke("stop_agent").catch(() => undefined);
  }

  function начатьЗаново() {
    setРеплики([]);
    история.current = [];
    setПричина(null);
    setЖдёт(null);
  }

  return (
    <div className="panel">
      <h2>Секретарь</h2>

      {инструменты && (
        <p className="muted">
          Инструментов: {инструменты.tools.map(название).join(", ") || "нет"}.
          {инструменты.can_change
            ? " Часть из них меняет файлы — такие действия спрашиваются."
            : " Пока только чтение: ничего изменить нельзя."}
        </p>
      )}

      {/*
        Режим исполнения виден и переключается здесь же.

        Раньше он был написан в настройках, но прочитать его можно было
        только в отдельной вкладке, а человек сюда приходит, когда
        агент уже что-то сделал не так. Переключатель рядом с разговором
        — единственное место, где его ищут в этот момент.

        Подтверждение при выборе режима «не спрашивать ничего»
        запрашивается дважды: это единственный переключатель, который
        снимает все вопросы разом, и молчаливое применение означало бы,
        что человек его не выбирал.
      */}
      {инструменты && (
        <div className="agent-modes">
          <span>
            Режим: <b>{инструменты.исполнитель ?? "—"}</b>
          </span>
          <button
            type="button"
            onClick={() =>
              сменитьРежим(() =>
                invoke<ToolsReport>("agent_set_executor", {
                  executor: инструменты.исполнитель === "Выполнять" ? "dry-run" : "live",
                }),
              )
            }
          >
            {инструменты.исполнитель === "Выполнять" ? "только показывать" : "выполнять"}
          </button>
          <span className="muted">{инструменты.подтверждения}</span>
          <button
            type="button"
            onClick={() => {
              const следующий =
                инструменты.подтверждения?.startsWith("Не спрашивать")
                  ? "actions"
                  : "never";
              const без_вопросов = следующий === "never";
              const применить = (подтверди?: boolean) =>
                сменитьРежим(() =>
                  invoke<ToolsReport>("agent_set_approval", {
                    mode: следующий,
                    подтверди: подтверди ?? null,
                  }),
                );
              if (без_вопросов) {
                if (window.confirm("Снимать все подтверждения? Секретарь сможет менять файлы и запускать программы, не спрашивая.")) {
                  применить(true);
                }
              } else {
                применить();
              }
            }}
          >
            {инструменты.подтверждения?.startsWith("Не спрашивать")
              ? "спрашивать о рискованном"
              : "не спрашивать ничего"}
          </button>
          <button
            type="button"
            onClick={() =>
              сменитьРежим(async () => {
                await invoke<string[]>("agent_reload_skills");
                return (await invoke<ToolsReport>("agent_tools")) as ToolsReport;
              })
            }
          >
            перечитать навыки
          </button>
        </div>
      )}

      {/* Навыки — процедуры, написанные человеком. Показываются
          списком, а не выпадающим: человек должен видеть, что именно
          секретарь умеет выполнять целиком. */}
      {инструменты && (инструменты.навыки?.length ?? 0) > 0 && (
        <div className="agent-skills">
          <b>Навыки</b>
          <ul>
            {инструменты.навыки!.map((н) => (
              <li key={н}>{н}</li>
            ))}
          </ul>
          <p className="muted">
            Навык — это готовая процедура. Скажи «выполни навык
            …», и секретарь пойдёт по шагам.
          </p>
        </div>
      )}

      {/* Сломанный навык не прячется: молчание выглядит как «навыков
          нет», и человек будет искать причину не там. */}
      {инструменты && (инструменты.сломанные_навыки?.length ?? 0) > 0 && (
        <div className="agent-broken">
          <b>Не прочитаны навыки</b>
          <ul>
            {инструменты.сломанные_навыки!.map((о, i) => (
              <li key={i}>{о}</li>
            ))}
          </ul>
        </div>
      )}

      {инструменты && инструменты.tools.length === 0 && (
        <p className="error">
          Ни одного инструмента не разрешено. Отметь хотя бы чтение файлов
          во вкладке «Знакомство».
        </p>
      )}

      <div className="agent-log">
        {реплики.length === 0 && (
          <p className="muted">
            Спроси что-нибудь про файлы в папках, которые ты разрешил.
            Например: «что лежит в папке с документами?»
          </p>
        )}

        {реплики.map((р, i) => (
          <div key={i} className={`agent-line agent-${р.роль}`}>
            {р.инструмент ? <b>{название(р.инструмент)}</b> : null}
            {р.роль === "отказ" && !р.инструмент ? <b>отказ</b> : null}
            <span>{р.текст}</span>
            {р.основание && (
              <span className="agent-basis">{р.основание}</span>
            )}
          </div>
        ))}

        {крутится && <div className="agent-line agent-wait">секретарь работает…</div>}
        <div ref={низ} />
      </div>

      {ждёт && !ждёт.отвечено && (
        <div className="agent-pending">
          <b>Нужно ваше решение</b>
          <p>{ждёт.pending.reason}</p>
          <p className="muted">
            Инструмент: {название(ждёт.pending.tool)} (
            {ждёт.pending.capability})
          </p>
          <pre>{JSON.stringify(ждёт.pending.arguments, null, 2)}</pre>
          <div className="agent-decide">
            {/*
              Три кнопки, а не две: «на всю задачу» означает, что
              следующие такие действия вопросов не вызовут. Название
              говорит именно об этом, а не «разрешить всё» — потому
              что разрешить всё нельзя.
            */}
            <button type="button" onClick={() => решить("один-раз")}>
              только этот раз
            </button>
            <button type="button" onClick={() => решить("всю-задачу")}>
              на всю задачу
            </button>
            <button type="button" className="agent-deny" onClick={() => решить("отказ")}>
              запретить
            </button>
          </div>
        </div>
      )}

      {причина && <p className="muted agent-stop">{причина}</p>}

      {/* Сжатие видно всегда, когда оно случилось: молчаливый
          разговор, который секретарь «забыл», выглядит как его
          память, а это ложь. */}
      {сжато && <p className="agent-compact">{сжато}</p>}

      <div className="onb-input">
        <input
          type="text"
          value={вопрос}
          placeholder="Спроси секретаря…"
          onChange={(e) => setВопрос(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              спросить();
            }
          }}
        />
        {крутится ? (
          <button type="button" onClick={остановить}>
            остановить
          </button>
        ) : (
          <button type="button" onClick={спросить} disabled={!вопрос.trim()}>
            спросить
          </button>
        )}
        {реплики.length > 0 && (
          <button type="button" onClick={начатьЗаново}>
            заново
          </button>
        )}
      </div>
    </div>
  );
}