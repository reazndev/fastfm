import { Routes } from '@angular/router';

import { activeGuard, adminGuard, loggedOutGuard, pendingGuard } from './guards';

export const routes: Routes = [
  {
    path: '',
    title: 'Home',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/home/home').then((m) => m.HomePage),
  },
  {
    path: 'keys',
    title: 'API keys',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/keys/keys').then((m) => m.KeysPage),
  },
  {
    path: 'demo',
    title: 'Speed comparison',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/demo/demo').then((m) => m.DemoPage),
  },
  {
    path: 'demo/live',
    title: 'Live race',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/race/race').then((m) => m.RacePage),
  },
  {
    path: 'settings',
    title: 'Settings',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/settings/settings').then((m) => m.SettingsPage),
  },
  {
    path: 'admin',
    title: 'Admin',
    canActivate: [adminGuard],
    loadComponent: () => import('./pages/admin/admin').then((m) => m.AdminPage),
  },
  {
    path: 'authorize',
    title: 'Allow access',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/authorize/authorize').then((m) => m.AuthorizePage),
  },
  {
    path: 'lastfm/callback',
    title: 'Connecting Last.fm',
    canActivate: [activeGuard],
    loadComponent: () => import('./pages/lastfm-callback').then((m) => m.LastfmCallbackPage),
  },
  {
    path: 'login',
    title: 'Log in',
    canActivate: [loggedOutGuard],
    loadComponent: () => import('./pages/auth/login').then((m) => m.LoginPage),
  },
  {
    path: 'signup',
    title: 'Create account',
    canActivate: [loggedOutGuard],
    loadComponent: () => import('./pages/auth/signup').then((m) => m.SignupPage),
  },
  {
    path: 'pending',
    title: 'Waiting for approval',
    canActivate: [pendingGuard],
    loadComponent: () => import('./pages/pending').then((m) => m.PendingPage),
  },
  { path: '**', redirectTo: '' },
];
