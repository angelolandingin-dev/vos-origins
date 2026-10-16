import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import App from './app/App';
import { AppDataProvider } from './app/providers';
import { initTheme } from './app/theme';
import './styles/globals.css';

/** Apply the persisted/OS theme before first render so there is no flash. */
initTheme();

/** Vite entry point; state lives behind `AppDataProvider`. */
const container = document.getElementById('root');
if (!container) {
  throw new Error('Root container #root was not found in index.html');
}

createRoot(container).render(
  <StrictMode>
    <AppDataProvider>
      <App />
    </AppDataProvider>
  </StrictMode>,
);
