import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { Icon } from "./Icons";
import type { Notice, Severity } from "../lib/types";

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

export function Card({
  title,
  action,
  children,
  flush = false,
  className = "",
}: {
  title?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  flush?: boolean;
  className?: string;
}) {
  return (
    <section className={`card${flush ? " card--flush" : ""} ${className}`}>
      {title !== undefined && (
        <header
          className="card__h"
          style={flush ? { padding: "var(--s4) var(--s5) 0" } : undefined}
        >
          <span className="card__t">{title}</span>
          {action}
        </header>
      )}
      {children}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="sw"
      disabled={disabled}
      onClick={() => onChange(!checked)}
    />
  );
}

export function Option({
  label,
  help,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  help?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="opt">
      <div className="opt__t">
        <div className="opt__l">{label}</div>
        {help && <div className="opt__h">{help}</div>}
      </div>
      <Switch checked={checked} onChange={onChange} label={label} disabled={disabled} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chip
// ---------------------------------------------------------------------------

export function Chip({
  tone,
  children,
  dark,
}: {
  tone?: "a" | "ok" | "warn" | "bad";
  children: ReactNode;
  dark?: boolean;
}) {
  return (
    <span className={`chip${tone ? ` chip--${tone}` : ""}${dark ? " chip--dark" : ""}`}>
      {children}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Notice
// ---------------------------------------------------------------------------

const noticeIcon: Record<Severity, (p: { size?: number }) => ReactNode> = {
  info: Icon.Info,
  warning: Icon.Warning,
  blocker: Icon.Warning,
};

export function NoticeBlock({ notice }: { notice: Notice }) {
  const Glyph = noticeIcon[notice.severity];
  const tone =
    notice.severity === "blocker"
      ? " note--bad"
      : notice.severity === "warning"
        ? " note--warn"
        : "";
  return (
    <div className={`note${tone}`}>
      <Glyph size={15} />
      <div>
        <div className="note__t">{notice.title}</div>
        <div className="note__b">{notice.detail}</div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

export function Blank({
  icon: Glyph = Icon.Box,
  title,
  children,
  action,
}: {
  icon?: (p: { size?: number }) => ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="blank">
      <span className="blank__i">
        <Glyph size={24} />
      </span>
      <div className="blank__t">{title}</div>
      {children && <p className="blank__b">{children}</p>}
      {action && <div style={{ marginTop: "var(--s3)" }}>{action}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

export function Modal({
  title,
  onClose,
  children,
  footer,
  wide = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  const surface = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    surface.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="scrim"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className={`modal${wide ? " modal--w" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        ref={surface}
      >
        <header className="modal__h">
          <h2>{title}</h2>
          <button
            type="button"
            className="btn btn--g btn--i btn--s"
            onClick={onClose}
            aria-label="Close"
          >
            <Icon.Close size={15} />
          </button>
        </header>
        <div className="modal__b">{children}</div>
        {footer && <footer className="modal__f">{footer}</footer>}
      </div>
    </div>
  );
}

export function Confirm({
  title,
  body,
  confirmLabel = "Confirm",
  destructive = false,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: ReactNode;
  confirmLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button type="button" className="btn btn--g" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className={`btn ${destructive ? "btn--bad" : "btn--a"}`}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </>
      }
    >
      <div className="col">{body}</div>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

type Tone = "success" | "error" | "info";

interface Toast {
  id: number;
  tone: Tone;
  title: string;
  detail?: string;
}

interface ToastApi {
  success: (title: string, detail?: string) => void;
  error: (title: string, detail?: string) => void;
  info: (title: string, detail?: string) => void;
  run: <T>(
    label: string,
    task: () => Promise<T>,
    describe?: (value: T) => string | undefined,
  ) => Promise<T | undefined>;
}

const ToastContext = createContext<ToastApi | null>(null);

export function useToast(): ToastApi {
  const api = useContext(ToastContext);
  if (!api) throw new Error("useToast must be used inside <ToastProvider>");
  return api;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);

  const push = useCallback((tone: Tone, title: string, detail?: string) => {
    const id = nextId.current++;
    setToasts((current) => [...current, { id, tone, title, detail }]);
    // Errors linger: they usually contain a path worth reading.
    window.setTimeout(
      () => setToasts((current) => current.filter((t) => t.id !== id)),
      tone === "error" ? 9000 : 4000,
    );
  }, []);

  const api = useMemo<ToastApi>(
    () => ({
      success: (title, detail) => push("success", title, detail),
      error: (title, detail) => push("error", title, detail),
      info: (title, detail) => push("info", title, detail),
      run: async (label, task, describe) => {
        try {
          const value = await task();
          push("success", label, describe?.(value));
          return value;
        } catch (error) {
          push("error", label, error instanceof Error ? error.message : String(error));
          return undefined;
        }
      },
    }),
    [push],
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="toasts" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast toast--${toast.tone}`}>
            <span className="toast__i">
              {toast.tone === "success" ? (
                <Icon.Check size={15} />
              ) : toast.tone === "error" ? (
                <Icon.Warning size={15} />
              ) : (
                <Icon.Info size={15} />
              )}
            </span>
            <div style={{ minWidth: 0 }}>
              <div className="toast__t">{toast.title}</div>
              {toast.detail && <div className="toast__b">{toast.detail}</div>}
            </div>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// Small pieces
// ---------------------------------------------------------------------------

export function Stat({ value, label }: { value: ReactNode; label: string }) {
  return (
    <div>
      <div className="stat__v">{value}</div>
      <div className="stat__k">{label}</div>
    </div>
  );
}

export function Spinner() {
  return <span className="spin" aria-label="Working" />;
}

export function Skeleton({
  variant = "line",
  width,
  count = 1,
}: {
  variant?: "line" | "block" | "tile";
  width?: string;
  count?: number;
}) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          className={`sk sk--${variant}`}
          style={width ? { width } : undefined}
          aria-hidden="true"
        />
      ))}
    </>
  );
}

export function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="btn btn--g btn--s btn--i"
      aria-label="Copy"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setDone(true);
          window.setTimeout(() => setDone(false), 1500);
        } catch {
          setDone(false);
        }
      }}
    >
      {done ? <Icon.Check size={13} /> : <Icon.Copy size={13} />}
    </button>
  );
}

/** Counts up when it appears, so figures read as live rather than printed. */
export function Num({ value }: { value: number }) {
  const [shown, setShown] = useState(0);
  const from = useRef(0);

  useEffect(() => {
    const start = performance.now();
    const origin = from.current;
    const delta = value - origin;
    if (delta === 0) {
      setShown(value);
      return;
    }
    let frame = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / 700, 1);
      setShown(origin + delta * (1 - (1 - t) ** 3));
      if (t < 1) frame = requestAnimationFrame(tick);
      else from.current = value;
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value]);

  return <>{Math.round(shown).toLocaleString()}</>;
}
