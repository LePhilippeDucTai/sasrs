/* M34.10 : PROC CLUSTER — OUTTREE= dendrogram dataset (small dataset).
   Five points in 1-D-ish space; Ward linkage. OUTTREE= writes one row per node
   (5 leaves + 4 merges = 9), columns _NAME_/_PARENT_/_NCL_/_FREQ_/_HEIGHT_ +
   the VAR coordinates; _HEIGHT_ is monotone non-decreasing across merges. */
title 'PROC CLUSTER OUTTREE=';
data pts;
  input name $ x y;
datalines;
P1 1 1
P2 1 2
P3 8 8
P4 9 8
P5 9 9
;
proc cluster data=pts method=average outtree=tree;
  var x y;
  id name;
run;
proc print data=tree noobs; run;
title;
