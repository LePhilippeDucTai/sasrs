/* PROC GLM — ANOVA à un facteur, OUTPUT OUT= (p=, r=).
   Cf. SAS/STAT 9.4 — The GLM Procedure. Valeurs ajustées = moyennes de
   groupe, recalculées indépendamment en Python 3. */
libname ind 'data';

proc glm data=ind.drug;
  class drug;
  model absorb = drug;
  output out=pred p=phat r=resid;
run;
