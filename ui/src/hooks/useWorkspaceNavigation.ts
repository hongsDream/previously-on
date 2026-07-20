import { useCallback, useState } from 'react';

export type WorkspaceView = 'overview' | 'task' | 'settings';
export type OverviewFocus = 'tasks' | 'sessions';

export function useWorkspaceNavigation() {
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>('task');
  const [overviewFocus, setOverviewFocus] = useState<OverviewFocus>('tasks');
  const [contextPackExpanded, setContextPackExpanded] = useState(() => (
    typeof window.matchMedia !== 'function' || !window.matchMedia('(max-width: 900px)').matches
  ));
  const [mobileInspectorOpen, setMobileInspectorOpen] = useState(true);

  const openTask = useCallback(() => setWorkspaceView('task'), []);
  const openOverview = useCallback((focus: OverviewFocus) => {
    setOverviewFocus(focus);
    setWorkspaceView('overview');
    setMobileInspectorOpen(false);
  }, []);
  const openSettings = useCallback(() => {
    setWorkspaceView('settings');
    setMobileInspectorOpen(false);
  }, []);
  const showContextPack = useCallback(() => {
    setWorkspaceView('task');
    setContextPackExpanded(true);
  }, []);
  const toggleContextPack = useCallback(() => {
    setContextPackExpanded((expanded) => !expanded);
  }, []);
  const showEvidence = useCallback(() => {
    setWorkspaceView('task');
    setMobileInspectorOpen(true);
  }, []);
  const openInspector = useCallback(() => setMobileInspectorOpen(true), []);
  const closeInspector = useCallback(() => setMobileInspectorOpen(false), []);

  return {
    workspaceView,
    overviewFocus,
    contextPackExpanded,
    mobileInspectorOpen,
    openTask,
    openOverview,
    openSettings,
    showContextPack,
    toggleContextPack,
    showEvidence,
    openInspector,
    closeInspector,
  };
}
