/* PROC REG — régression linéaire simple, OUTEST= et OUTPUT OUT= (p=, r=).
   Cf. SAS/STAT 9.4 User's Guide — The REG Procedure (syntax MODEL, OUTEST=,
   OUTPUT). Statistiques recalculées indépendamment en Python 3 (MCO). */
libname ind 'data';

proc reg data=ind.sample outest=est;
  model yield = fert;
  output out=pred p=yhat r=resid;
run;
