import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App.tsx';
import { AppProviders } from '@/providers/app-providers';
import './index.css';

const app = (
  <AppProviders>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </AppProviders>
);

// StrictMode double-invokes effects, which starts and stops the desktop
// websocket twice per mount. Useful in development, wrong for a release build.
createRoot(document.getElementById('root')!).render(
  import.meta.env.DEV ? <StrictMode>{app}</StrictMode> : app
);
