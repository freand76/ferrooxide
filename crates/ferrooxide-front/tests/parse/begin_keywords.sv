// `begin_keywords / `end_keywords: names that are keywords of the set in effect are listed
`begin_keywords "1364-2005"
module old(input bit, output logic);
  wire small;
  assign logic = ~bit;
endmodule
`end_keywords

`begin_keywords "1364-1995"
module older;
  reg config, uwire, int;
endmodule
`end_keywords

module m;
  reg bit, config;
endmodule
