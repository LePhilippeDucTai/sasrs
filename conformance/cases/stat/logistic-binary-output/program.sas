/* PROC LOGISTIC — régression logistique binaire, OUTPUT OUT= predicted=.
   Cf. SAS/STAT 9.4 — The LOGISTIC Procedure. MLE recalculé indépendamment
   en Python 3 (Newton-Raphson). */
libname ind 'data';

proc logistic data=ind.trial descending;
  model resp = dose;
  output out=pred predicted=phat;
run;
