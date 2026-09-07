import React, { useState, useEffect } from 'react';
import { getTodayPlan, getUpcomingPlanning, getConflicts } from '../api/planning';
import { TodayPlan, UpcomingPlanning, Conflict } from '../api/types';

type PlanningTab = 'today' | 'upcoming' | 'plan' | 'conflicts';

export const PlanningScreen: React.FC = () => {
  const [activeTab, setActiveTab] = useState<PlanningTab>('today');
  const [todayPlan, setTodayPlan] = useState<TodayPlan | null>(null);
  const [upcoming, setUpcoming] = useState<UpcomingPlanning | null>(null);
  const [conflicts, setConflicts] = useState<Conflict[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [todayRes, upcomingRes, conflictsRes] = await Promise.all([
        getTodayPlan().catch(() => null),
        getUpcomingPlanning(7).catch(() => null),
        getConflicts().catch(() => []),
      ]);
      setTodayPlan(todayRes);
      setUpcoming(upcomingRes);
      setConflicts(conflictsRes);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Failed to load planning data');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    queueMicrotask(() => {
      void loadData();
    });
  }, []);

  const getFeasibilityBadge = (state?: string) => {
    switch (state) {
      case 'feasible':
        return <span className="px-3 py-1 text-xs font-semibold bg-emerald-500/20 text-emerald-400 rounded-full border border-emerald-500/30">Feasible</span>;
      case 'likely_feasible':
        return <span className="px-3 py-1 text-xs font-semibold bg-blue-500/20 text-blue-400 rounded-full border border-blue-500/30">Likely Feasible</span>;
      case 'uncertain':
        return <span className="px-3 py-1 text-xs font-semibold bg-amber-500/20 text-amber-400 rounded-full border border-amber-500/30">Uncertain</span>;
      case 'infeasible':
        return <span className="px-3 py-1 text-xs font-semibold bg-rose-500/20 text-rose-400 rounded-full border border-rose-500/30">Infeasible</span>;
      default:
        return null;
    }
  };

  return (
    <div className="flex flex-col h-full bg-slate-950 text-slate-100 p-4 space-y-4 overflow-y-auto">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-slate-800 pb-3">
        <div>
          <h1 className="text-xl font-bold tracking-wide text-white">Personal Planning Engine</h1>
          <p className="text-xs text-slate-400">Unified schedule, commitments, deadlines & feasibility</p>
        </div>
        <button
          onClick={loadData}
          disabled={loading}
          className="px-3 py-1.5 text-xs font-medium bg-indigo-600 hover:bg-indigo-500 active:bg-indigo-700 text-white rounded-lg transition-colors disabled:opacity-50"
        >
          {loading ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>

      {error && (
        <div className="p-3 text-xs bg-rose-500/10 border border-rose-500/30 text-rose-400 rounded-xl">
          {error}
        </div>
      )}

      {/* Tabs */}
      <div className="flex bg-slate-900/80 p-1 rounded-xl border border-slate-800 text-xs font-medium">
        {(['today', 'upcoming', 'plan', 'conflicts'] as PlanningTab[]).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={`flex-1 py-2 text-center rounded-lg capitalize transition-colors ${
              activeTab === tab
                ? 'bg-indigo-600 text-white shadow font-semibold'
                : 'text-slate-400 hover:text-slate-200'
            }`}
          >
            {tab}
            {tab === 'conflicts' && conflicts.length > 0 && (
              <span className="ml-1.5 px-1.5 py-0.5 text-[10px] bg-rose-500 text-white rounded-full">
                {conflicts.length}
              </span>
            )}
          </button>
        ))}
      </div>

      {/* Loading state */}
      {loading && !todayPlan && (
        <div className="flex flex-col items-center justify-center py-12 text-slate-500 text-xs space-y-2">
          <div className="w-6 h-6 border-2 border-indigo-500 border-t-transparent rounded-full animate-spin"></div>
          <span>Computing deterministic plan...</span>
        </div>
      )}

      {/* TODAY TAB */}
      {activeTab === 'today' && todayPlan && (
        <div className="space-y-4">
          {/* Feasibility Overview Card */}
          <div className="p-4 bg-slate-900 border border-slate-800 rounded-2xl space-y-3 shadow-sm">
            <div className="flex items-center justify-between">
              <span className="text-xs uppercase tracking-wider font-semibold text-slate-400">Schedule Feasibility</span>
              {getFeasibilityBadge(todayPlan.feasibility?.state)}
            </div>
            <p className="text-sm font-medium text-slate-200">{todayPlan.feasibility?.explanation}</p>
            <div className="grid grid-cols-2 gap-2 pt-2 text-xs border-t border-slate-800/80 text-slate-400">
              <div>
                Available Focus: <span className="font-semibold text-slate-200">{todayPlan.total_available_minutes} mins</span>
              </div>
              <div>
                Known Effort: <span className="font-semibold text-slate-200">{todayPlan.feasibility?.total_known_effort_minutes || 0} mins</span>
              </div>
            </div>
          </div>

          {/* Commitments Section */}
          <div className="space-y-2">
            <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">Fixed Commitments Today</h2>
            {todayPlan.commitments.length === 0 ? (
              <div className="p-3 text-xs text-slate-500 bg-slate-900/40 rounded-xl border border-slate-800/50">
                No fixed appointments or reminders today.
              </div>
            ) : (
              todayPlan.commitments.map((c) => (
                <div key={c.id} className="p-3 bg-slate-900 border border-slate-800 rounded-xl flex items-center justify-between">
                  <div>
                    <h3 className="text-xs font-semibold text-slate-100">{c.title}</h3>
                    <p className="text-[10px] text-slate-400">Source: {c.source}</p>
                  </div>
                  <span className="text-xs font-mono text-indigo-400 bg-indigo-500/10 px-2 py-1 rounded-md">
                    {c.is_all_day ? 'All Day' : `${new Date(c.start_time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`}
                  </span>
                </div>
              ))
            )}
          </div>

          {/* Recommended Work Blocks */}
          <div className="space-y-2">
            <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">Recommended Focus Blocks</h2>
            {todayPlan.recommended_blocks.length === 0 ? (
              <div className="p-4 text-xs text-center text-slate-500 bg-slate-900/40 rounded-xl border border-slate-800/50">
                No active tasks to schedule today. Enjoy your free time!
              </div>
            ) : (
              todayPlan.recommended_blocks.map((block, idx) => (
                <div key={idx} className="p-3 bg-slate-900 border border-slate-800 rounded-xl space-y-1.5">
                  <div className="flex items-center justify-between">
                    <h3 className="text-xs font-bold text-indigo-300">{block.title}</h3>
                    <span className="text-[10px] font-mono bg-slate-800 text-slate-300 px-2 py-0.5 rounded-md">
                      {block.duration_minutes} mins
                    </span>
                  </div>
                  <p className="text-[11px] text-slate-400">{block.rationale}</p>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* UPCOMING TAB */}
      {activeTab === 'upcoming' && upcoming && (
        <div className="space-y-4">
          <div className="p-4 bg-slate-900 border border-slate-800 rounded-2xl space-y-2">
            <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">7-Day Workload Outlook</h2>
            <div className="space-y-2 pt-2">
              {upcoming.daily_workload_minutes.map(([day, mins]) => (
                <div key={day} className="flex items-center justify-between text-xs border-b border-slate-800/50 pb-1">
                  <span className="text-slate-300 font-mono">{day}</span>
                  <span className="font-semibold text-slate-200">{mins} mins</span>
                </div>
              ))}
            </div>
          </div>

          <div className="space-y-2">
            <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">Upcoming Deadlines</h2>
            {upcoming.deadlines.length === 0 ? (
              <div className="p-3 text-xs text-slate-500 bg-slate-900/40 rounded-xl border border-slate-800/50">
                No upcoming deadlines in the next 7 days.
              </div>
            ) : (
              upcoming.deadlines.map((item) => (
                <div key={item.id} className="p-3 bg-slate-900 border border-slate-800 rounded-xl flex items-center justify-between">
                  <div>
                    <h3 className="text-xs font-semibold text-slate-100">{item.title}</h3>
                    <span className="text-[10px] text-slate-400 uppercase tracking-wider">{item.priority} Priority</span>
                  </div>
                  {item.deadline && (
                    <span className="text-xs font-mono text-rose-400 bg-rose-500/10 px-2 py-1 rounded-md">
                      {new Date(item.deadline.due_at).toLocaleDateString()}
                    </span>
                  )}
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* PLAN TAB */}
      {activeTab === 'plan' && todayPlan && (
        <div className="space-y-3">
          <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">Allocated Plan Blocks</h2>
          {todayPlan.recommended_blocks.length === 0 ? (
            <div className="p-4 text-xs text-center text-slate-500 bg-slate-900/40 rounded-xl border border-slate-800/50">
              No plan generated. Add active tasks or reminders to build a plan.
            </div>
          ) : (
            todayPlan.recommended_blocks.map((b, i) => (
              <div key={i} className="p-3 bg-slate-900 border border-slate-800 rounded-xl space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-slate-100">{b.title}</span>
                  <span className="text-[10px] font-mono text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded-md">
                    Confidence: {(b.confidence * 100).toFixed(0)}%
                  </span>
                </div>
                <p className="text-[11px] text-slate-400">{b.rationale}</p>
                <div className="text-[10px] text-slate-500 font-mono">
                  {new Date(b.start_time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} - {new Date(b.end_time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                </div>
              </div>
            ))
          )}
        </div>
      )}

      {/* CONFLICTS TAB */}
      {activeTab === 'conflicts' && (
        <div className="space-y-3">
          <h2 className="text-xs uppercase tracking-wider font-semibold text-slate-400">Detected Schedule Conflicts</h2>
          {conflicts.length === 0 ? (
            <div className="p-4 text-xs text-center text-emerald-400 bg-emerald-500/10 border border-emerald-500/30 rounded-xl">
              Zero conflicts detected! All deadlines and commitments are in balance.
            </div>
          ) : (
            conflicts.map((c, i) => (
              <div
                key={i}
                className={`p-3 rounded-xl border space-y-1 ${
                  c.severity === 'critical'
                    ? 'bg-rose-500/10 border-rose-500/30 text-rose-300'
                    : 'bg-amber-500/10 border-amber-500/30 text-amber-300'
                }`}
              >
                <div className="flex items-center justify-between text-xs font-bold uppercase tracking-wider">
                  <span>{c.conflict_type.replace(/_/g, ' ')}</span>
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-slate-950/60 font-mono">
                    {c.severity}
                  </span>
                </div>
                <p className="text-xs">{c.reason}</p>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
};
