import { useEffect } from 'react';

import { AppShell } from '../components/AppShell';
import { PromotionDialog } from '../components/PromotionDialog';
import { Distribute } from '../pages/Distribute';
import { IntegrationHistory } from '../pages/IntegrationHistory';
import { Promote } from '../pages/Promote';
import { Reconcile } from '../pages/Reconcile';
import { Settings } from '../pages/Settings';
import { useApp } from './context';

let fixtureNoticeShown = false;
let updateNoticeShown = false;

export default function App() {
  const app = useApp();
  const { route, pushNotice, dismissNotice, openSettings, checkForUpdates } = app;
  const authStatus = app.auth.data;

  // One silent check per session: populates the persistent header update pill
  // and shows a brief ~5s toast when a release exists. Never installs anything
  // on its own, and stays quiet when the endpoint is unreachable.
  useEffect(() => {
    if (updateNoticeShown) return;
    updateNoticeShown = true;
    void checkForUpdates().then((info) => {
      if (!info) return;
      pushNotice({
        tone: 'info',
        title: `Version ${info.version} is available`,
        message: 'Click to open Settings › Updates.',
        onClick: () => {
          dismissNotice();
          openSettings('updates');
        },
      });
    });
  }, [checkForUpdates, dismissNotice, openSettings, pushNotice]);

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
        availableUpdate={app.availableUpdate}
        onOpenUpdates={() => openSettings('updates')}
        notice={app.notice}
        onDismissNotice={app.dismissNotice}
      >
        {route === 'promote' ? <Promote /> : null}
        {route === 'distribute' ? <Distribute /> : null}
        {route === 'reconcile' ? <Reconcile /> : null}
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
        excludedPaths={app.promotion.excludedPaths}
        onAddExcludedPath={app.addPromotionExcludedPath}
        onSetExcludedPaths={app.setPromotionExcludedPaths}
        dependencies={app.promotion.dependencies}
        dependenciesLoading={app.promotion.dependenciesLoading}
        preflight={app.promotion.preflight}
        preflightLoading={app.promotion.preflightLoading}
        dispatching={app.promotion.dispatching}
        progress={app.promotion.progress}
        error={app.promotion.error}
        result={app.promotion.result}
        onDispatch={app.dispatchPromotion}
        onOpenExternal={app.openExternal}
      />
    </>
  );
}
