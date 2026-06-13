/* M10 : MEANS avec WEIGHT sur un petit jeu construit (verifiable a la main).
   x=[1,2,3], w=[1,2,3] -> SumWgt=6, Sum=Swx=14, Mean=14/6, Var=CSS_w/(n-1)=(10/3)/2=5/3. */
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
