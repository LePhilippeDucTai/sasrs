/* M34.2 : PROC TTEST — options déférées : BY, SIDES=, colonnes CI.
   Données : sashelp.class.

   1) BY sex sur un test 1-échantillon de height vs H0=60.
      Le jeu doit être trié par sex (F puis M).
        F (n=9) : mean=60.589, std=5.018, se=1.673, t=0.352, df=8.
        M (n=10): mean=63.910, std=4.938, se=1.561, t=2.504, df=9.

   2) SIDES=U (test unilatéral supérieur) sur le 1-échantillon global.
      height vs H0=60 : t=1.9867, df=18.
        Pr > t (unilatéral) = Pr(|t|>..)/2 = 0.0624/2 ≈ 0.0312.

   3) CI=95 (colonnes de limites de confiance) sur le 1-échantillon global.
      height vs H0=60 : mean=62.3368, se=1.1762, df=18, t_{0.975,18}=2.10092.
        95% CL Mean = 62.3368 ± 2.10092*1.1762 = [59.866, 64.808].
        95% CL Std (chi2_{.975,18}=31.526, chi2_{.025,18}=8.231) :
          [5.1271*sqrt(18/31.526), 5.1271*sqrt(18/8.231)] = [3.873, 7.580]. */
libname d 'data';

/* Le jeu sashelp.class est déjà trié par name ; on trie par sex pour BY. */
proc sort data=d.class out=class_by;
  by sex;
run;

title 'PROC TTEST BY sex: height vs H0=60';
proc ttest data=class_by h0=60;
  var height;
  by sex;
run;

title 'PROC TTEST SIDES=U: height vs H0=60 (one-sided upper)';
proc ttest data=d.class h0=60 sides=u;
  var height;
run;

title 'PROC TTEST CI columns: height vs H0=60 (CI=95)';
proc ttest data=d.class h0=60 ci=95;
  var height;
run;

title;
