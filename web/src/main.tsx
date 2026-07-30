import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import '@/styles/tokens.css';
import '@/styles/base.css';
import '@/styles/components.css';

import { App } from '@/app/App';

const container = document.getElementById('root');
if (container === null) {
  throw new Error('index.html is missing the #root element');
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
