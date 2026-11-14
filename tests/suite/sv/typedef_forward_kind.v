// exit: 1
// a forward typedef names a kind (enum here) that the definition does not have: an error
module t;
  typedef enum T;
  typedef struct packed { int x; } T;
  initial $display("FAILED");
endmodule
