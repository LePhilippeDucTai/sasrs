/* M34.3 : PROC NPAR1WAY — score methods (Median/Savage/Van der Waerden) +
   exact Wilcoxon, on sashelp.class (height by sex).

   Oracle cross-checks (k=2 → one-way χ² equals the UNCORRECTED two-sample Z²;
   the printed two-sample Z carries SAS's 0.5 continuity correction):
     - Wilcoxon statistic = 73 (rank sum of F), Mean Under H0 = 90,
       Std Dev = 12.2367, corrected Z = -1.3484 (matches m24 snapshot).
       Uncorrected Z0 = (73-90)/12.2367 = -1.3892 → Kruskal-Wallis
       Chi-Square ≈ 1.9298, Pr > ChiSq ≈ 0.1647.
     - All score methods give a NEGATIVE two-sample Z for F (F shorter than M),
       and each One-Way χ² ≈ (that method's uncorrected Z)².
     - n = 19 ≤ 30 so the exact Wilcoxon distribution is enumerated; the exact
       two-sided p should sit close to the normal approximation (0.1775). */
libname d 'data';

title 'NPAR1WAY scores: height by sex (Wilcoxon/Median/Savage/Normal)';
proc npar1way data=d.class wilcoxon median savage normal;
  class sex;
  var height;
run;

title 'NPAR1WAY exact Wilcoxon: height by sex';
proc npar1way data=d.class wilcoxon exact;
  class sex;
  var height;
run;
title;
