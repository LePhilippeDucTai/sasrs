/* M10 : MEANS avec WEIGHT sur un petit jeu construit (verifiable a la main).
   x=[1,2,3], w=[1,2,3] -> SumWgt=6, Sum=Swx=14, Mean=14/6, et depuis J03-P2
   (VARDEF=DF -> diviseur W-1=5) Var=CSS_w/5=(10/3)/5=2/3, Std=0.8164965809. */
data wtest;
  x = 1; w = 1; output;
  x = 2; w = 2; output;
  x = 3; w = 3; output;
run;

title 'Weighted statistics (weight w)';
proc means data=wtest n mean std sum;
  weight w;
  var x;
run;
