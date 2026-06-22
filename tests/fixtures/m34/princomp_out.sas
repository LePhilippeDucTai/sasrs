/* M34.9 : PROC PRINCOMP — OUT= component scores on sashelp.class.
   The eigenvalues appear in the listing; each component score column Prin_j has
   sample variance equal to eigenvalue_j and mean ≈ 0 (correlation-based PCA).
   OUT= dataset = all input columns + Prin1 Prin2 Prin3. */
libname d 'data';
title 'PROC PRINCOMP OUT= component scores';
proc princomp data=d.class out=pcout;
  var height weight age;
run;
proc print data=pcout noobs; run;
title;
