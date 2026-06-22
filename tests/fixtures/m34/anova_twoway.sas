/* M34.5 : PROC ANOVA — two-way model with interaction (sex | agegrp).
   sashelp.class split into age groups; the sex×agegrp design is UNBALANCED
   (Old: F=4,M=5 ; Yng: F=5,M=5), so Type I SS (sequential) ≠ Type III SS
   (partial) for the first-entered effect.

   Oracle cross-checks:
     - Class Level Information lists BOTH sex (F M) and agegrp (Old Yng).
     - Model DF = 3 (sex 1 + agegrp 1 + sex*agegrp 1), Error DF = 19-4 = 15.
     - Type I SS of sex + agegrp + sex*agegrp = Model SS.
     - The last-entered term (sex*agegrp) has Type I == Type III.
     - Each term F = (term SS / term DF) / MSE. */
libname d 'data';

data class2;
  set d.class;
  if age >= 14 then agegrp = 'Old';
  else agegrp = 'Yng';
run;

title 'PROC ANOVA two-way: weight = sex agegrp sex*agegrp';
proc anova data=class2;
  class sex agegrp;
  model weight = sex agegrp sex*agegrp;
run;

title;
