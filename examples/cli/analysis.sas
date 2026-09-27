/* J06-P3 — Exemple CLI autonome : analyse d'un CSV avec sasrs.             */
/*                                                                          */
/* Usage (depuis la racine du dépôt) :                                      */
/*   cargo run --locked --bin sasrs -- examples/cli/analysis.sas            */
/*                                                                          */
/* Les chemins relatifs du programme résolvent sous le répertoire du        */
/* fichier .sas (examples/cli/) : le CSV d'entrée est ../data/patients.csv  */
/* et le résumé produit est écrit dans ../out/summary.csv (créé au besoin). */
/*                                                                          */
/* Sortie attendue :                                                        */
/*   - code retour 0 (aucune ERROR/WARNING dans la log, écrite sur stderr); */
/*   - listing sur stdout : PROC PRINT de la table WORK.SUMMARY avec les    */
/*     deux lignes F et M (voir tests/examples.rs) ;                        */
/*   - ../out/summary.csv :                                                 */
/*       Sex,N_PATIENTS,MEAN_AGE                                            */
/*       F,3,33                                                             */
/*       M,2,48                                                             */
/*     (un champ numérique 33.0 peut s'écrire 33 selon le backend CSV —     */
/*     le test normalise les valeurs numériques).                           */

* Import du CSV patients (en-tête = noms de variables). ;
proc import datafile='../data/patients.csv'
  out=work.patients
  dbms=csv
  replace;
  getnames=yes;
run;

* Résumé par sexe : effectif et âge moyen. ;
* NB : on référence les colonnes avec leur casse canonique (Sex, Age) —
  PROC SQL est sensible à la casse des identifiants sur les tables issues
  de PROC IMPORT (bug connu, cf. rapport J06-P3). ;
proc sql;
  create table work.summary as
  select Sex,
         count(*) as n_patients,
         mean(Age) as mean_age
  from work.patients
  group by Sex;
quit;

* Exporter le résumé en CSV (le répertoire ../out doit préexister). ;
proc export data=work.summary
  outfile='../out/summary.csv'
  dbms=csv
  replace;
run;

* Afficher le résumé dans le listing. ;
title 'Resume des patients par sexe';
proc print data=work.summary;
run;
