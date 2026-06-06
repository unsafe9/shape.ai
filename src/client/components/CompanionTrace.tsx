import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { fetchMcpTrace, type McpClientInfo, type McpTraceEvent, type McpTraceKind } from "../lib/api";

type CompanionTraceProps = {
  open: boolean;
  clients: McpClientInfo[];
  onClose: () => void;
};

const POLL_INTERVAL_MS = 4_000;
const DEFAULT_LIMIT = 50;

const KIND_LABELS: Record<McpTraceKind, string> = {
  read: "read",
  write: "write",
  comment: "comment",
  export: "export",
  "proposal-created": "proposed",
  "proposal-accepted": "accepted",
  "proposal-rejected": "rejected",
  error: "error"
};

function relativeTime(at: number): string {
  const deltaS = Math.floor((Date.now() - at) / 1000);
  if (deltaS < 5) return "just now";
  if (deltaS < 60) return `${deltaS}s ago`;
  const deltaM = Math.floor(deltaS / 60);
  if (deltaM < 60) return `${deltaM}m ago`;
  const deltaH = Math.floor(deltaM / 60);
  return `${deltaH}h ago`;
}

export function CompanionTrace({ open, clients, onClose }: CompanionTraceProps) {
  const [activeClientId, setActiveClientId] = useState<string | null>(null);
  const [events, setEvents] = useState<McpTraceEvent[]>([]);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const pollRef = useRef<number | null>(null);

  const targetClientId = activeClientId ?? clients[0]?.clientId ?? null;

  const fetchTrace = useCallback(async (clientId: string) => {
    try {
      const result = await fetchMcpTrace(clientId, DEFAULT_LIMIT);
      setEvents(result.trace);
      setFetchError(null);
    } catch (err) {
      setFetchError(err instanceof Error ? err.message : "Trace fetch failed");
    }
  }, []);

  useEffect(() => {
    if (!open || !targetClientId) {
      setEvents([]);
      return;
    }

    void fetchTrace(targetClientId);

    pollRef.current = window.setInterval(() => {
      void fetchTrace(targetClientId);
    }, POLL_INTERVAL_MS);

    return () => {
      if (pollRef.current !== null) window.clearInterval(pollRef.current);
      pollRef.current = null;
    };
  }, [open, targetClientId, fetchTrace]);

  if (!open) return null;

  const activeClient = clients.find((c) => c.clientId === targetClientId) ?? clients[0] ?? null;

  return (
    <aside
      id="companion-trace"
      className="companion-trace-drawer"
      aria-label="Agent activity trail"
      onPointerDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <div className="companion-trace-head">
        <div>
          <span>Agent Trace</span>
          <strong>{activeClient?.label ?? "No client"}</strong>
        </div>
        <button className="icon-button" type="button" onClick={onClose} aria-label="Close agent trace" title="Close agent trace">
          <X size={15} />
        </button>
      </div>

      {clients.length > 1 && (
        <div className="companion-trace-filter" role="list" aria-label="Filter by client">
          {clients.map((client) => (
            <button
              key={client.clientId}
              type="button"
              role="listitem"
              className={`companion-trace-chip ${client.clientId === targetClientId ? "is-active" : ""}`}
              style={{ "--chip-color": client.color } as React.CSSProperties}
              onClick={() => setActiveClientId(client.clientId)}
              title={`${client.label} v${client.version}`}
              aria-pressed={client.clientId === targetClientId}
            >
              <span
                className="companion-trace-chip-icon"
                style={{ background: client.color }}
              >
                {(client.label[0] ?? "?").toUpperCase()}
              </span>
              <span className="companion-trace-chip-label">{client.label}</span>
            </button>
          ))}
        </div>
      )}

      {fetchError ? (
        <div className="companion-trace-error" role="alert">{fetchError}</div>
      ) : events.length === 0 ? (
        <div className="companion-trace-empty">No activity yet</div>
      ) : (
        <ol className="companion-trace-list" aria-label="Recent operations">
          {events.map((event, index) => (
            <TraceRow
              key={event.operationId ?? `read-${index}-${event.at}`}
              event={event}
              clientColor={activeClient?.color ?? "#888"}
            />
          ))}
        </ol>
      )}
    </aside>
  );
}

type TraceRowProps = {
  event: McpTraceEvent;
  clientColor: string;
};

function TraceRow({ event, clientColor }: TraceRowProps) {
  const kindLabel = KIND_LABELS[event.kind] ?? event.kind;
  const isError = event.kind === "error";
  const isPending = event.kind === "proposal-created";

  return (
    <li className={`companion-trace-row ${isError ? "is-error" : ""} ${isPending ? "is-pending" : ""}`}>
      <span
        className="companion-trace-kind-dot"
        style={{ background: isError ? "var(--bad)" : clientColor }}
        aria-hidden="true"
      />
      <div className="companion-trace-content">
        <span className="companion-trace-kind">{kindLabel}</span>
        <span className="companion-trace-summary">{event.summary}</span>
        {event.errorMessage && (
          <span className="companion-trace-error-msg">{event.errorMessage}</span>
        )}
      </div>
      <time className="companion-trace-time" dateTime={new Date(event.at).toISOString()} title={new Date(event.at).toLocaleString()}>
        {relativeTime(event.at)}
      </time>
    </li>
  );
}
