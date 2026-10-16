import { useEffect } from 'react';

import { AppShell } from '../components/AppShell';
import { PromotionDialog } from '../components/PromotionDialog';
import { CommitBrowser } from '../pages/CommitBrowser';
import { Dashboard } from '../pages/Dashboard';
import { IntegrationHistory } from '../pages/IntegrationHistory';
import { PromoteCommits } from '../pages/PromoteCommits';
import { Settings } from '../pages/Settings';
import { Subsystems } from '../pages/Subsystems';
import { WorkflowRuns } from '../pages/WorkflowRuns';
import { useApp } from './context';

/** Shell + active screen + the app-level promotion dialog shared by multiple screens. */
let fixtureNoticeShown = false;

export default function App() {
  const app = useApp();
  const { route, pushNotice } = app;
  const authStatus = app.auth.data;

  // A fixture-mode banner must be visible exactly once per session start. The
  // module-level flag survives the dev server's double-invoked mount, which
  // otherwise shows the banner twice.
  useEffect(() => {
    if (authStatus?.dataSource !== 'fixture') return;
    if (fixtureNoticeShown) return;
    fixtureNoticeShown = true;
    pushNotice({
      tone: 'info',
      title: 'Fixture data mode',
      message: 'Sample records. GitHub CLI detection is live.',
    });
  }, [authStatus?.dataSource, pushNotice]);

  const handleSwitchAccount = () => {
    app.pushNotice({
      tone: 'info',
      title: 'Switch GitHub account in your terminal',
      message: 'Run `gh auth switch`, then Re-check GitHub CLI status.',
    });
  };

  return (
    <>
      <AppShell
        route={route}
        onNavigate={app.navigate}
        auth={app.auth}
        onRefreshAuth={app.refreshAuth}
        onSwitchAccount={handleSwitchAccount}
        targetBranch={app.targetBranch}
        selectedSubsystemName={app.selectedSubsystem?.name ?? null}
        notice={app.notice}
        onDismissNotice={app.dismissNotice}
      >
        {route === 'dashboard' ? <Dashboard /> : null}
        {route === 'subsystems' ? <Subsystems /> : null}
        {route === 'commit-browser' ? <CommitBrowser /> : null}
        {route === 'promote-commits' ? <PromoteCommits /> : null}
        {route === 'workflow-runs' ? <WorkflowRuns /> : null}
        {route === 'integration-history' ? <IntegrationHistory /> : null}
        {route === 'settings' ? <Settings /> : null}
      </AppShell>

      <PromotionDialog
        open={app.promotion.open}
        onClose={app.closePromotionDialog}
        subsystem={app.selectedSubsystem}
        sourceShas={app.promotion.shas}
        onRemoveSha={app.removePromotionSha}
        commits={app.promotion.commits}
        commitsLoading={app.promotion.commits.length === 0}
        targetBranches={app.targetBranches.data}
        targetBranch={app.targetBranch}
        onTargetBranchChange={app.setTargetBranch}
        mode={app.promotion.mode}
        onModeChange={app.setPromotionMode}
        baseSha={app.promotion.baseSha}
        onBaseShaChange={app.setPromotionBaseSha}
        delivery={app.promotion.delivery}
        onDeliveryChange={app.setPromotionDelivery}
        message={app.promotion.message}
        onMessageChange={app.setPromotionMessage}
        allowGaps={app.promotion.allowGaps}
        onAllowGapsChange={app.setPromotionAllowGaps}
        excludedPaths={app.promotion.excludedPaths}
        onAddExcludedPath={app.addPromotionExcludedPath}
        onRemoveExcludedPath={app.removePromotionExcludedPath}
        dependencies={app.promotion.dependencies}
        dependenciesLoading={app.promotion.dependenciesLoading}
        preflight={app.promotion.preflight}
        preflightLoading={app.promotion.preflightLoading}
        dispatching={app.promotion.dispatching}
        error={app.promotion.error}
        result={app.promotion.result}
        onDispatch={app.dispatchPromotion}
        onOpenExternal={app.openExternal}
        onViewRuns={() => {
          app.closePromotionDialog();
          app.navigate('workflow-runs');
        }}
      />
    </>
  );
}
