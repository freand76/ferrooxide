// exit: 1
// an unpacked array port of a struct type connected to an array of another struct type
typedef struct packed { logic [3:0] a; logic [3:0] b; } r_t;
typedef struct packed { logic [3:0] x; logic [3:0] y; } s_t;
module m (input r_t req_i [2]);
endmodule
module top;
  s_t req [2];
  m u(.req_i(req));
endmodule
