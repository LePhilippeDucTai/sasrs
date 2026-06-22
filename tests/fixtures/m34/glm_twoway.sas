/* M34.5 : PROC GLM — two-way model with interaction + SOLUTION + LSMEANS.
   Same unbalanced sex×agegrp design on weight. GLM reports Type I and Type III
   SS (which differ here), reference-cell parameter estimates, and least-squares
   means (marginal, averaged uniformly over the other factor's levels). */
libname d 'data';

data class2;
  set d.class;
  if age >= 14 then agegrp = 'Old';
  else agegrp = 'Yng';
run;

title 'PROC GLM two-way: weight = sex agegrp sex*agegrp';
proc glm data=class2;
  class sex agegrp;
  model weight = sex agegrp sex*agegrp / solution;
  lsmeans sex agegrp / se;
run;

title;
