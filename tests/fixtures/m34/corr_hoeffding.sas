/* M34.1 : PROC CORR — Hoeffding's D + Spearman pondéré (WEIGHT).
   Données : sashelp.class (height, weight, age).

   Oracle Hoeffding's D (≡ SAS PROC CORR HOEFFDING, 5 décimales) :
     D(height,weight) = 0.31609
     D(height,age)    = 0.18856
     D(weight,age)    = 0.20579
   La diagonale D(v,v) est la dépendance maximale atteignable pour n=19
   (SAS imprime cette valeur, pas un 1.00000 forcé).
   Prob > D : approximation asymptotique de Blum-Kiefer-Rosenblatt (Imhof) —
   documentée comme approchée pour petit n ; le coefficient D est exact.

   Oracle Spearman pondéré : avec des poids entiers = comptes de réplication,
   le Spearman pondéré est exactement le Spearman ordinaire sur les données
   répliquées (rangs moyens pondérés ; cf. tests unitaires). Ici le poids est
   un comptage d'observations (weight n'altère pas les Simple Statistics). */
libname d 'data';

title 'CORR Hoeffding D on sashelp.class';
proc corr data=d.class hoeffding nosimple;
  var height weight age;
run;

title 'CORR weighted Spearman (weight = age) on sashelp.class';
proc corr data=d.class spearman nosimple;
  var height weight;
  weight age;
run;
