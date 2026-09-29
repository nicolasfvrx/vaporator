# Vaporator

Bot Discord en Rust pour suivre les builds et les actualités officielles des applications Steam.

## État du projet

Le dépôt est initialisé. L'implémentation de la V1 reste à réaliser selon le [plan](docs/PLAN.md).

## Développement

- `master` accueillera la V1 fonctionnelle.
- `dev` est la branche de développement : chaque modification cohérente et vérifiée doit être commitée puis poussée sur `origin/dev`.
- Le merge de `dev` vers `master` interviendra lorsque la V1 sera fonctionnelle.

## Cible V1

Un Discord, configuration par commandes slash, suivi Steam via PICS, actualités officielles, stockage SQLite et déploiement Docker sous Linux.
