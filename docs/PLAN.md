# Plan V1 — Vaporator

## Objectif

Créer un bot Discord en Rust pour une communauté, configurable par commandes slash administrateur. Annoncer les nouveaux builds Steam et les publications officielles des développeurs. Héberger le bot sur Linux avec Docker et conserver l'état dans SQLite sur un volume persistant.

## Architecture

- Tokio pour l'exécution asynchrone, Poise/Serenity pour Discord, steam-vent pour Steam, reqwest pour les actualités et SQLx pour SQLite.
- Séparer collecte Steam, détection des événements, stockage et publication Discord.
- Première étape : valider les requêtes PICS et la lecture des branches/builds des quatre applications DayZ. Vérifier les noms, les accès anonymes et les éventuels besoins d'authentification.
- Essayer une connexion Steam anonyme par défaut. Fournir `vaporator steam-login` pour une authentification locale avec Steam Guard et une session persistante protégée dans le volume Docker. Aucun identifiant Steam dans Discord.

## Surveillance des builds

- Garder une connexion Steam persistante et interroger les changements PICS toutes les 60 secondes à partir du dernier numéro mémorisé.
- Relire les applications suivies concernées et annoncer les changements de `buildid` de leur branche, y compris un retour à un ancien build.
- Ignorer les changements de métadonnées sans changement de build.
- Au premier ajout, enregistrer l'état courant sans annonce historique.
- Après une interruption, comparer avec le dernier état conservé. Ne pas promettre de reconstituer les builds intermédiaires.
- Relire toutes les applications suivies après reconnexion et périodiquement pour réconcilier les états.

## Commandes Discord

Commandes réservées aux administrateurs du Discord configuré :

| Commande | Fonction |
| --- | --- |
| `/steam suivre` | AppID, builds/actualités/les deux, branche, salon et rôle facultatif |
| `/steam modifier` | Modifier un suivi, dont sa source d'actualités |
| `/steam retirer` | Supprimer un suivi |
| `/steam liste` | Lister les suivis et leurs identifiants |
| `/steam branches` | Lister les branches accessibles d'une application |
| `/steam dayz` | Installer le préréglage DayZ |
| `/steam statut` | Connexions, dernières vérifications et erreurs |
| `/steam test` | Envoyer un exemple sans mention dans le salon choisi |

Préréglage DayZ : branche `public` pour les quatre applications suivantes.

| Application | AppID |
| --- | --- |
| DayZ | 221100 |
| DayZ Experimental | 1024020 |
| DayZ Server | 223350 |
| DayZ Experimental Server | 1042420 |

Une annonce de build affiche l'application, la branche, l'ancien et le nouveau build et l'heure de détection. Messages en français. Mentions désactivées par défaut et limitées au rôle explicitement configuré.

## Actualités

- Consulter `ISteamNews/GetNewsForApp` toutes les 5 minutes et filtrer les publications officielles Steam Community.
- Publier titre, court extrait nettoyé et lien original, dans la langue de l'article.
- Autoriser une application source distincte pour les actualités d'un serveur dédié.
- Le préréglage DayZ suit les actualités des deux applications clientes sans les répéter pour les serveurs.
- Séparer annonces de builds et articles sans association automatique supposée.
- Au premier ajout, mémoriser les articles existants sans les envoyer. Après interruption, rattraper les nouveaux articles des dernières 24 heures avec pagination.
- Dédupliquer les articles par identifiant Steam et salon.

## Persistance et erreurs

- Enregistrer changements détectés et notifications à envoyer dans une même transaction SQLite.
- Réessayer les envois après erreur, respecter les limites Discord et reconnecter Steam avec un délai progressif.
- Afficher les problèmes de permissions et de session Steam dans les logs et `/steam statut`.
- Un doublon reste possible si Discord accepte un message avant une coupure empêchant d'enregistrer son succès. Ne pas promettre une livraison exactement une fois.

## Livraison et validation

- Projet Rust, Dockerfile, Docker Compose, configuration d'exemple et guide de création du bot, permissions, Steam Guard et sauvegarde du volume.
- Tests : changement de build, retour arrière, métadonnées seules, premier lancement silencieux, redémarrage et reprise des notifications.
- Tests actualités : filtrage officiel, doublons, pagination et contenu trop long.
- Tests erreurs : application inconnue, branche inaccessible, permissions Discord insuffisantes, coupures Steam/Discord et expiration de session.
- Compilation, tests, Clippy, démarrage Docker, lecture réelle des quatre applications et message de test dans le salon configuré.

## Limites V1

Un seul Discord, applications configurables au-delà de DayZ, branches accessibles sans mot de passe. Le suivi Workshop, l'installation et le redémarrage des serveurs de jeu sont hors périmètre.

## Références

- [steam-vent](https://docs.rs/steam-vent/latest/steam_vent/)
- [Requêtes PICS dans SteamKit](https://github.com/SteamRE/SteamKit/blob/master/SteamKit2/SteamKit2/Steam/Handlers/SteamApps/SteamApps.cs)
- [Actualités Steam](https://partner.steamgames.com/doc/webapi/ISteamNews)
- [Hébergement des serveurs DayZ](https://community.bistudio.com/wiki/DayZ:Hosting_a_Linux_Server)
