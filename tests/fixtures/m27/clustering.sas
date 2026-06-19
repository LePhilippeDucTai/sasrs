title 'M27.3: PROC DISTANCE, PROC CLUSTER, PROC FASTCLUS';
data work.pts;
  input x;
datalines;
1
2
3
7
8
9
;

proc distance data=work.pts out=work.dist method=euclid;
  var x;
run;

proc cluster data=work.pts method=ward;
  var x;
run;

proc fastclus data=work.pts maxclusters=2 out=work.clust maxiter=20;
  var x;
run;
run;
title;
